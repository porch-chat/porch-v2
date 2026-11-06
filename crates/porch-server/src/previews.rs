//! Decode only bounded raster images off the reactor, never original SVG/HTML.
use image::{ImageFormat, ImageReader, Limits};
use std::io::{self, Cursor, Read};
pub const MAX_PREVIEW_SOURCE: u64 = 16 * 1024 * 1024;
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "image preview unavailable")
}
/// Caller holds a bounded blocking-work permit. Memory limits are best effort;
/// strict 4096px dimensions and a 16MiB encoded input bound are also enforced.
pub fn thumbnail(mut source: impl Read) -> io::Result<Vec<u8>> {
    let mut input = Vec::new();
    source
        .by_ref()
        .take(MAX_PREVIEW_SOURCE + 1)
        .read_to_end(&mut input)?;
    if input.len() as u64 > MAX_PREVIEW_SOURCE {
        return Err(invalid());
    }
    let mut reader = ImageReader::new(Cursor::new(input)).with_guessed_format()?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
    ) {
        return Err(invalid());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| invalid())?.thumbnail(480, 320);
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| invalid())?;
    Ok(output.into_inner())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_reencode_and_reject_active_content() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1000, 600)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let preview = thumbnail(Cursor::new(png.into_inner())).unwrap();
        let decoded = image::load_from_memory(&preview).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (480, 288));
        for bad in [
            b"<svg onload=alert(1)>".as_slice(),
            b"<html>bad</html>",
            b"\x89PNG\r\n\x1a\ntruncated",
        ] {
            assert!(thumbnail(Cursor::new(bad)).is_err());
        }
    }
    #[test]
    fn excessive_dimensions_are_rejected() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(4097, 1)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        assert!(thumbnail(Cursor::new(png.into_inner())).is_err());
    }
}

/// Optional Linux FFmpeg poster extraction, never playback or remote URLs.
/// Requires explicit administrator-selected absolute executable paths. Resource
/// limits and FFmpeg protocol allowlists are defense in depth, not an OS sandbox.
#[derive(Clone)]
pub struct VideoPreviewer {
    binary: std::path::PathBuf,
    limiter: std::path::PathBuf,
    permits: std::sync::Arc<tokio::sync::Semaphore>,
}
pub const MAX_VIDEO_SOURCE: u64 = 64 * 1024 * 1024;
impl VideoPreviewer {
    pub fn new(binary: std::path::PathBuf, limiter: std::path::PathBuf) -> io::Result<Self> {
        if !cfg!(target_os = "linux")
            || !binary.is_absolute()
            || !limiter.is_absolute()
            || !binary.is_file()
            || !limiter.is_file()
        {
            return Err(invalid());
        }
        Ok(Self {
            binary,
            limiter,
            permits: std::sync::Arc::new(tokio::sync::Semaphore::new(2)),
        })
    }
    pub async fn poster(&self, file: std::fs::File) -> io::Result<Vec<u8>> {
        use std::process::Stdio;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _permit = self.permits.try_acquire().map_err(|_| invalid())?;
        if file.metadata()?.len() > MAX_VIDEO_SOURCE {
            return Err(invalid());
        }
        // Input is a pipe, not a pathname or URI. Neither nested references nor
        // subtitles/audio/scripts/playlists are allowed to open other resources.
        let mut child = tokio::process::Command::new(&self.limiter)
            .args([
                "--as=536870912",
                "--cpu=4",
                "--nofile=32",
                "--stack=8388608",
                "--",
            ])
            .arg(&self.binary)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-max_alloc",
                "33554432",
                "-threads",
                "1",
                "-filter_threads",
                "1",
                "-protocol_whitelist",
                "pipe",
                "-format_whitelist",
                "mov,matroska,webm",
                "-codec_whitelist",
                "h264,hevc,vp8,vp9,av1",
                "-max_pixels",
                "8388608",
                "-probesize",
                "1048576",
                "-analyzeduration",
                "1000000",
                "-i",
                "pipe:0",
                "-map",
                "0:v:0",
                "-an",
                "-sn",
                "-dn",
                "-frames:v",
                "1",
                "-vf",
                "scale=480:320:force_original_aspect_ratio=decrease",
                "-c:v",
                "png",
                "-threads",
                "1",
                "-f",
                "image2pipe",
                "pipe:1",
            ])
            .env_clear()
            .env("OMP_NUM_THREADS", "1")
            .env("OPENBLAS_NUM_THREADS", "1")
            .env("MKL_NUM_THREADS", "1")
            // Bound glibc per-thread arena reservations within RLIMIT_AS.
            .env("MALLOC_ARENA_MAX", "2")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut input = child.stdin.take().ok_or_else(invalid)?;
        let mut output = child
            .stdout
            .take()
            .ok_or_else(invalid)?
            .take(2 * 1024 * 1024 + 1);
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let write = async {
                let mut source = tokio::fs::File::from_std(file).take(MAX_VIDEO_SOURCE + 1);
                // A one-frame decoder normally exits before reading the full
                // video. BrokenPipe is expected; other write errors are not.
                if let Err(e) = tokio::io::copy(&mut source, &mut input).await
                    && e.kind() != io::ErrorKind::BrokenPipe
                {
                    return Err(e);
                }
                let _ = input.shutdown().await;
                drop(input);
                Ok::<_, io::Error>(())
            };
            let read = async {
                let mut bytes = Vec::new();
                output.read_to_end(&mut bytes).await?;
                if bytes.len() > 2 * 1024 * 1024 {
                    return Err(invalid());
                }
                Ok::<_, io::Error>(bytes)
            };
            let (_, bytes) = tokio::try_join!(write, read)?;
            if !child.wait().await?.success() {
                return Err(invalid());
            }
            // Validate/re-encode even the decoder's output before returning it.
            tokio::task::spawn_blocking(move || thumbnail(std::io::Cursor::new(bytes)))
                .await
                .map_err(|_| invalid())?
        })
        .await;
        match result {
            Ok(Ok(bytes)) => Ok(bytes),
            _ => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(invalid())
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod video_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires maintained Linux ffmpeg/prlimit; CI installs explicitly"]
    async fn video_poster_resource_and_input_guards() {
        use std::io::Write;
        let decoder =
            VideoPreviewer::new("/usr/bin/ffmpeg".into(), "/usr/bin/prlimit".into()).unwrap();
        assert!(VideoPreviewer::new("ffmpeg".into(), "/usr/bin/prlimit".into()).is_err());
        let file = tempfile::tempfile().unwrap();
        file.set_len(MAX_VIDEO_SOURCE + 1).unwrap();
        assert!(decoder.poster(file).await.is_err());
        for data in [
            b"<html><script>no</script></html>".as_slice(),
            b"#EXTM3U\nhttp://127.0.0.1/private",
            b"ffconcat version 1.0\nfile '/etc/passwd'",
            b"\x00\x00\x00\x18ftypisomtruncated",
        ] {
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(data).unwrap();
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start(0)).unwrap();
            assert!(decoder.poster(file).await.is_err());
        }
        let _one = decoder.permits.acquire().await.unwrap();
        let _two = decoder.permits.acquire().await.unwrap();
        assert!(decoder.poster(tempfile::tempfile().unwrap()).await.is_err());
    }
}
