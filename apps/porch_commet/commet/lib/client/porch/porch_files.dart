import 'dart:async';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;
import 'package:commet/cache/file_provider.dart';
import 'package:commet/client/attachment.dart';
import 'package:flutter/painting.dart';
import 'package:porch_flutter/bridge.dart';

/// A local file queued for upload. [temporary] files are ours and deleted after the send.
class PorchProcessedFile implements ProcessedAttachment {
  final String path;
  final bool temporary;
  PorchProcessedFile(this.path, {this.temporary = false});
}

/// Authenticated preview fetched through the Rust bridge; Dart never touches the network.
class PorchPreviewImage extends ImageProvider<PorchPreviewImage> {
  final PorchBridge bridge;
  final int conversation, message;
  final String attachment;
  const PorchPreviewImage(
      this.bridge, this.conversation, this.message, this.attachment);

  @override
  Future<PorchPreviewImage> obtainKey(ImageConfiguration configuration) =>
      Future.value(this);

  @override
  ImageStreamCompleter loadImage(
          PorchPreviewImage key, ImageDecoderCallback decode) =>
      MultiFrameImageStreamCompleter(codec: _load(decode), scale: 1);

  Future<ui.Codec> _load(ImageDecoderCallback decode) async {
    final Uint8List bytes =
        await bridge.attachmentPreview(conversation, message, attachment);
    return decode(await ui.ImmutableBuffer.fromUint8List(bytes));
  }

  @override
  bool operator ==(Object other) =>
      other is PorchPreviewImage &&
      other.conversation == conversation &&
      other.message == message &&
      other.attachment == attachment;
  @override
  int get hashCode => Object.hash(conversation, message, attachment);
}

/// Downloads through the bridge into a new file; never overwrites.
class PorchFileProvider implements FileProvider {
  final PorchBridge bridge;
  final String cacheDir;
  final int conversation, message;
  final String attachment, filename;
  PorchFileProvider(this.bridge, this.cacheDir, this.conversation,
      this.message, this.attachment, this.filename);

  @override
  String get fileIdentifier => 'porch:$conversation/$message/$attachment';
  @override
  Stream<DownloadProgress>? get onProgressChanged => null;

  static String safeName(String name) {
    final base = name.split(RegExp(r'[\\/]')).last;
    final clean = base.replaceAll(RegExp(r'[^A-Za-z0-9._ -]'), '_');
    return clean.isEmpty || clean.startsWith('.') ? 'file' : clean;
  }

  Future<File> _cached() async {
    final dir = Directory('$cacheDir/downloads/${safeName(attachment)}');
    final file = File('${dir.path}/${safeName(filename)}');
    if (!await file.exists()) {
      await dir.create(recursive: true);
      await bridge.downloadAttachment(
          conversation, message, attachment, file.path);
    }
    return file;
  }

  @override
  Future<Uri?> resolve() async => (await _cached()).uri;
  @override
  Future<Uint8List?> getFileData() async => (await _cached()).readAsBytes();
  @override
  Future<void> save(String filepath) =>
      bridge.downloadAttachment(conversation, message, attachment, filepath);
}

/// Builds Commet attachments from a protocol message's attachment list.
List<Attachment> porchAttachments(PorchBridge bridge, String cacheDir,
    int conversation, int message, List<dynamic> raw) {
  final out = <Attachment>[];
  for (final a in raw.cast<Map<String, dynamic>>()) {
    final id = a['id'] as String;
    final name = a['filename'] as String? ?? 'file';
    final mime = a['mime'] as String?;
    final size = (a['bytes'] as num?)?.toInt();
    final file = PorchFileProvider(
        bridge, cacheDir, conversation, message, id, name);
    if (mime != null && mime.startsWith('image/')) {
      out.add(ImageAttachment(
          PorchPreviewImage(bridge, conversation, message, id), file,
          name: name, mimeType: mime, fileSize: size));
    } else {
      out.add(FileAttachment(file,
          name: name, mimeType: mime, fileSize: size));
    }
  }
  return out;
}
