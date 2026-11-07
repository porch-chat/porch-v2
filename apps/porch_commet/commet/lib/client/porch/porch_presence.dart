import 'dart:async';
import 'package:commet/client/components/user_presence/user_presence_component.dart';
import 'package:commet/client/porch/porch_client.dart';

/// Read-only gateway presence. Users absent from the map are offline.
class PorchPresenceComponent implements UserPresenceComponent<PorchClient> {
  @override
  final PorchClient client;
  PorchPresenceComponent(this.client);
  final _changes = StreamController<(String, UserPresence)>.broadcast();
  Map<String, String> _map = {};

  static UserPresenceStatus statusOf(String? s) => switch (s) {
        'online' => UserPresenceStatus.online,
        'idle' || 'dnd' => UserPresenceStatus.unavailable,
        _ => UserPresenceStatus.offline,
      };

  /// Replaces the map and emits only users whose status changed.
  void update(Map<String, String> next) {
    final prev = _map;
    _map = next;
    for (final id in {...prev.keys, ...next.keys}) {
      if (prev[id] != next[id]) {
        _changes.add((id, UserPresence(statusOf(next[id]))));
      }
    }
  }

  @override
  Stream<(String, UserPresence)> get onPresenceChanged => _changes.stream;
  @override
  Future<UserPresence> getUserPresence(String userId) async =>
      UserPresence(statusOf(_map[userId]));
  @override
  bool get usePublicReadReceipts => false;
  @override
  Future<void> setUsePublicReadReceipts(bool value) async =>
      throw UnsupportedError('Read receipts are not supported yet');
  @override
  bool get typingIndicatorEnabled => false;
  @override
  Future<void> setTypingIndicatorEnabled(bool value) async =>
      throw UnsupportedError('Typing indicators are not supported yet');
  @override
  Future<void> setStatus(UserPresenceStatus status,
          {String? message, bool clearMessage = false}) async =>
      throw UnsupportedError('Setting status is not supported yet');
}
