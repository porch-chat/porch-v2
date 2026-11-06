import 'dart:convert';
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
}
