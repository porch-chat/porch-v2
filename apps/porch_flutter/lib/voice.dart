import 'dart:async';
import 'package:livekit_client/livekit_client.dart';
import 'package:logging/logging.dart' as logging;
import 'package:synchronized/synchronized.dart';
import 'bridge.dart';

/// Media only. Rust owns auth, permission checks, short-lived tickets and lease heartbeat.
class PorchVoice {
  final PorchBridge bridge;
  final Lock _lock = Lock();
  Room? _room;
  EventsListener<RoomEvent>? _listener;
  bool muted = true, deafened = false, canSpeak = false;
  bool get connected => _room != null;
  final void Function() changed;
  PorchVoice(this.bridge, this.changed) {
    // SDK signal URLs may contain short-lived credentials. Never attach a logger.
    logging.hierarchicalLoggingEnabled = true;
    disableLogging();
  }

  Future<void> join(int community, int channel) => _lock.synchronized(() async {
    await _leave();
    final ticket = await bridge.joinVoice(community, channel);
    final room = Room();
    _room = room;
    muted = true;
    deafened = false;
    canSpeak = ticket['can_speak'] == true;
    _listener = room.createListener()
      ..on<RoomDisconnectedEvent>((_) => unawaited(_end()))
      ..on<TrackSubscribedEvent>((_) => unawaited(_applyPlayout()));
    try {
      final transport = ticket['transport'] as Json;
      // Never save, log, or put these credentials in view snapshots.
      await room.connect(
        transport['url'] as String,
        transport['token'] as String,
      );
      await bridge.voiceFlags(true, false);
      changed();
    } catch (_) {
      await _leave();
      rethrow;
    }
  });

  Future<void> _end() async {
    try {
      await leave();
    } catch (_) {
      /* lease expiry is the fallback */
    }
  }

  Future<void> _applyPlayout() async {
    try {
      await _playout();
    } catch (_) {
      await _end();
    }
  }

  Future<void> _playout() async {
    for (final participant
        in _room?.remoteParticipants.values ?? <RemoteParticipant>[]) {
      for (final publication in participant.audioTrackPublications) {
        // Stop already buffered local audio immediately, then update SFU preferences.
        if (deafened) {
          await publication.track?.disable();
          await publication.disable();
        } else {
          await publication.track?.enable();
          await publication.enable();
        }
      }
    }
  }

  Future<void> flags(bool mute, bool deafen) => _lock.synchronized(() async {
    final room = _room;
    if (room == null) return;
    try {
      // Close microphone before any HTTP; only reopen after authorization succeeds.
      await room.localParticipant?.setMicrophoneEnabled(false);
      muted = mute || deafen || !canSpeak;
      deafened = deafen;
      await _playout();
      changed();
      await bridge.voiceFlags(muted, deafened);
      if (!muted) await room.localParticipant?.setMicrophoneEnabled(true);
    } catch (_) {
      await _leave();
      rethrow;
    }
    changed();
  });

  Future<void> leave() => _lock.synchronized(_leave);

  Future<void> _leave() async {
    final room = _room;
    final listener = _listener;
    _room = null;
    _listener = null;
    muted = true;
    deafened = false;
    canSpeak = false;
    try {
      await listener?.dispose();
      try {
        await room?.localParticipant?.setMicrophoneEnabled(false);
      } finally {
        try {
          await room?.disconnect();
        } finally {
          await room?.dispose();
        }
      }
    } finally {
      try {
        await bridge.leaveVoice();
      } finally {
        changed();
      }
    }
  }
}
