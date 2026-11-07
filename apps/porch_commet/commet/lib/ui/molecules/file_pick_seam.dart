import 'package:commet/client/attachment.dart';

/// Test seam for the desktop "File" attachment source. Null in production, so the
/// real platform file picker is used. Integration tests inject a picker here and
/// the picked files then travel the normal attach, process and upload path.
Future<List<PendingFileAttachment>?> Function()? filePickOverride;
