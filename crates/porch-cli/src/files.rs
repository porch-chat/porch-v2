//! Small synchronous transfer driver. Shared-core transfer UI is a later layer.
use crate::{Cli, Result, print_json};
use porch_protocol::*;
use reqwest::Method;
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

fn key(value: &str) -> Result<&str> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("invalid upload ID".into());
    }
    Ok(value)
}
fn source(path: &Path) -> Result<(File, u64, String, String)> {
    let mut file = File::open(path)?;
    let bytes = file.metadata()?.len();
    if !file.metadata()?.is_file() || !(1..=MAX_UPLOAD_BYTES).contains(&bytes) {
        return Err("source must be a regular file of 1 byte to 1 GiB".into());
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("filename must be UTF-8")?
        .to_owned();
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok((file, bytes, format!("{:x}", hash.finalize()), name))
}
fn response(response: reqwest::blocking::Response) -> Result<reqwest::blocking::Response> {
    if !response.status().is_success() {
        return Err(format!("file request failed with HTTP {}", response.status()).into());
    }
    Ok(response)
}
pub fn command(cli: &Cli, args: &[String]) -> Result<()> {
    match args[0].as_str() {
        "file-create" | "file-upload" | "file-resume" => {
            if args.len() != 3 {
                return Err("expected conversation-or-upload ID and source path".into());
            }
            let (mut file, bytes, hash, filename) = source(Path::new(&args[2]))?;
            let mut status: UploadStatus = if args[0] == "file-resume" {
                cli.request::<()>(
                    Method::GET,
                    &format!("/uploads/{}", key(&args[1])?),
                    None,
                    true,
                )?
                .json()?
            } else {
                cli.request(
                    Method::POST,
                    "/uploads",
                    Some(&CreateUpload {
                        conversation_id: args[1].parse()?,
                        filename,
                        bytes,
                        sha256: hash.clone(),
                    }),
                    true,
                )?
                .json()?
            };
            validate(&status, bytes, &hash)?;
            if args[0] == "file-create" {
                return print_json(&status);
            }
            // A failed process can be resumed with this public opaque ID.
            // stderr contains no bearer token or local filesystem path.
            eprintln!("upload {}", key(&status.id)?);
            while status.state == UploadState::Receiving && status.received < bytes {
                let offset = status.received;
                file.seek(SeekFrom::Start(offset))?;
                let length = (bytes - offset).min(UPLOAD_CHUNK_BYTES) as usize;
                let mut chunk = vec![0; length];
                file.read_exact(&mut chunk)?;
                let result = cli
                    .client
                    .post(format!(
                        "{}/uploads/{}/chunks?offset={offset}",
                        cli.base,
                        key(&status.id)?
                    ))
                    .bearer_auth(env::var("PORCH_TOKEN")?)
                    .body(chunk)
                    .send();
                // Never blindly retry a timed-out append. Query the committed
                // offset on a subsequent explicit invocation of file-resume.
                let next: UploadStatus = response(result?)?.json()?;
                validate(&next, bytes, &hash)?;
                if next.id != status.id || next.received != offset + length as u64 {
                    return Err("invalid upload acknowledgement".into());
                }
                status = next;
            }
            if status.state == UploadState::Receiving {
                status = cli
                    .request::<()>(
                        Method::POST,
                        &format!("/uploads/{}/complete", key(&status.id)?),
                        None,
                        true,
                    )?
                    .json()?;
                validate(&status, bytes, &hash)?;
            }
            if !matches!(status.state, UploadState::Ready | UploadState::Attached) {
                return Err("upload not finalized".into());
            }
            print_json(&status)?;
        }
        "file-status" | "file-complete" | "file-abort" => {
            if args.len() != 2 {
                return Err("expected upload ID".into());
            }
            let path = format!("/uploads/{}", key(&args[1])?);
            match args[0].as_str() {
                "file-status" => cli.read::<UploadStatus>(&path)?,
                "file-abort" => cli.delete(&path)?,
                _ => print_json(
                    &cli.request::<()>(Method::POST, &format!("{path}/complete"), None, true)?
                        .json::<UploadStatus>()?,
                )?,
            }
        }
        "file-download" => {
            if args.len() != 3 {
                return Err("expected attachment ID and NEW destination path".into());
            }
            let id = key(&args[1])?;
            // Refuse overwrite and partial files rather than exposing unverified
            // bytes as successful. A later core API adds durable ranged resume.
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut out = options.open(&args[2])?;
            let result = (|| -> Result<()> {
                let response = response(cli.request::<()>(
                    Method::GET,
                    &format!("/attachments/{id}"),
                    None,
                    true,
                )?)?;
                let declared = response
                    .content_length()
                    .ok_or("download lacks content length")?;
                if declared > MAX_UPLOAD_BYTES {
                    return Err("download too large".into());
                }
                let copied = std::io::copy(&mut response.take(MAX_UPLOAD_BYTES + 1), &mut out)?;
                if copied != declared {
                    return Err("truncated or oversized download".into());
                }
                out.flush()?;
                out.sync_all()?;
                Ok(())
            })();
            drop(out);
            if result.is_err() {
                std::fs::remove_file(&args[2])?;
            }
            result?;
            println!("ok");
        }
        _ => return Err("unknown file command".into()),
    }
    Ok(())
}
fn validate(status: &UploadStatus, bytes: u64, hash: &str) -> Result<()> {
    key(&status.id)?;
    if status.bytes != bytes
        || status.sha256 != hash
        || status.chunk_bytes != UPLOAD_CHUNK_BYTES
        || status.received > bytes
        || (status.received != bytes && !status.received.is_multiple_of(UPLOAD_CHUNK_BYTES))
    {
        return Err("upload does not match source or protocol".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upload_ids_cannot_inject_paths() {
        for bad in ["../uploads", "/x", "x?token=secret", "", &"A".repeat(32)] {
            assert!(key(bad).is_err());
        }
        assert!(key(&"a".repeat(32)).is_ok());
    }
}
