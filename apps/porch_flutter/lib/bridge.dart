import 'dart:convert';
import 'dart:typed_data';
import 'src/rust/api.dart' as rust;

typedef Json = Map<String, dynamic>;

/// UI-facing seam. Fake implementations belong in tests, never the app entrypoint.
abstract class PorchBridge {
  Stream<Json> snapshots();
  Future<void> login(
    String origin,
    String username,
    String password,
    String cache,
  );
  Future<void> logout();
  Future<void> select(int id);
  Future<void> send(int id, String text);
  Future<Json> joinVoice(int community, int channel);
  Future<void> leaveVoice();
  Future<void> voiceFlags(bool muted, bool deafened);
  Future<void> sendFiles(int id, List<String> paths, String text);
  Future<Uint8List> attachmentPreview(int conversation, int message, String attachment);
  Future<void> downloadAttachment(int conversation, int message, String attachment, String destination);
  Future<String> createCommunityInvite(int community, int maxUses, int expiresInSeconds);
  Future<void> joinCommunity(String code);
  Future<Json> voiceParticipants(int community, int channel);
}

class RustPorchBridge implements PorchBridge {
  @override
  Stream<Json> snapshots() =>
      rust.snapshots().map((value) => jsonDecode(value) as Json);
  @override
  Future<void> login(
    String origin,
    String username,
    String password,
    String cache,
  ) => rust.login(
    origin: origin,
    username: username,
    password: password,
    cacheRoot: cache,
  );
  @override
  Future<void> logout() => rust.logout();
  @override
  Future<void> select(int id) => rust.selectConversation(id: id);
  @override
  Future<void> send(int id, String text) =>
      rust.sendMessage(conversation: id, text: text);
  @override
  Future<Json> joinVoice(int community, int channel) async =>
      jsonDecode(await rust.joinVoice(community: community, channel: channel))
          as Json;
  @override
  Future<void> leaveVoice() => rust.leaveVoice();
  @override
  Future<void> voiceFlags(bool muted, bool deafened) =>
      rust.voiceFlags(muted: muted, deafened: deafened);
  @override
  Future<void> sendFiles(int id, List<String> paths, String text) =>
      rust.sendFiles(conversation: id, paths: paths, text: text);
  @override
  Future<Uint8List> attachmentPreview(int conversation, int message, String attachment) =>
      rust.attachmentPreview(conversation: conversation, message: message, attachment: attachment);
  @override
  Future<void> downloadAttachment(int conversation, int message, String attachment, String destination) =>
      rust.downloadAttachment(conversation: conversation, message: message, attachment: attachment, destination: destination);
  @override
  Future<String> createCommunityInvite(int community, int maxUses, int expiresInSeconds) =>
      rust.createCommunityInvite(community: community, maxUses: maxUses, expiresInSeconds: expiresInSeconds);
  @override
  Future<void> joinCommunity(String code) => rust.joinCommunity(code: code);
  @override
  Future<Json> voiceParticipants(int community, int channel) async =>
      jsonDecode(await rust.voiceParticipants(community: community, channel: channel)) as Json;
}
