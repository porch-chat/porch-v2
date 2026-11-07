import 'dart:async';
import 'package:commet/client/client.dart';
import 'package:commet/client/components/room_component.dart';
import 'package:commet/client/components/voip/voip_session.dart';
import 'package:commet/client/components/voip/voip_stream.dart';
import 'package:commet/client/components/voip_room/voip_room_component.dart';
import 'package:commet/client/porch/porch_client.dart';
import 'package:commet/client/porch/porch_room.dart';
import 'package:flutter/material.dart';
import 'package:flutter_webrtc/flutter_webrtc.dart';
import 'package:porch_flutter/voice.dart';

/// Audio-only session over the Porch LiveKit voice plane. Rust owns auth and leases.
class PorchVoipSession implements VoipSession {
  final PorchClient porch;
  final PorchRoom room;
  final PorchVoice voice;
  final _state = StreamController<void>.broadcast();
  final _conn = StreamController<VoipState>.broadcast();
  VoipState _s = VoipState.connecting;
  PorchVoipSession(this.porch, this.room, this.voice);

  void set(VoipState s) {
    _s = s;
    _conn.add(s);
    _state.add(null);
  }

  @override
  Client get client => porch;
  @override
  String get sessionId => room.identifier;
  @override
  String get roomId => room.identifier;
  @override
  String? get remoteUserId => null;
  @override
  String? get remoteUserName => null;
  @override
  String get roomName => room.displayName;
  @override
  VoipState get state => _s;
  @override
  bool get isMicrophoneMuted => voice.muted;
  @override
  bool get supportsScreenshare => false;
  @override
  bool get isSharingScreen => false;
  @override
  bool get isCameraEnabled => false;
  @override
  double get generalAudioLevel => 0;
  @override
  VoipStream? get remoteUserMediaStream => null;
  @override
  List<VoipStream> get streams => const [];
  @override
  Future<void> acceptCall({bool withMicrophone = false, bool withCamera = false}) async {}
  @override
  Future<void> declineCall() => hangUpCall();
  @override
  Future<void> hangUpCall() async {
    try {
      await voice.leave();
    } finally {
      set(VoipState.ended);
    }
  }

  @override
  Stream<VoipState> get onConnectionStateChanged => _conn.stream;
  @override
  Stream<void> get onStateChanged => _state.stream;
  @override
  Stream<void> get onUpdateVolumeVisualizers => const Stream.empty();
  @override
  Future<void> setMicrophoneMute(bool muted) async {
    await voice.flags(muted, voice.deafened);
    _state.add(null);
  }

  @override
  Future<void> updateStats() async {}
  @override
  Future<ScreenCaptureSource?> pickScreenCapture(BuildContext context) async => null;
  @override
  Future<void> setScreenShare(ScreenCaptureSource source) async =>
      throw UnsupportedError('Screen sharing is not supported yet');
  @override
  Future<void> stopScreenshare() async {}
  @override
  Future<void> setCamera(MediaDeviceInfo? device) async =>
      throw UnsupportedError('Camera is not supported yet');
  @override
  Future<void> stopCamera() async {}
}

class PorchVoipRoom extends RoomComponent<PorchClient, PorchRoom>
    implements VoipRoomComponent<PorchClient, PorchRoom> {
  PorchVoipRoom(super.client, super.room);
  late final PorchVoice _voice = PorchVoice(client.bridge, () {});
  PorchVoipSession? _session;
  List<String> _participants = [];
  Timer? _timer;
  late final StreamController<void> _changed =
      StreamController<void>.broadcast(onListen: _start, onCancel: _stop);

  void _start() {
    _refresh();
    _timer ??= Timer.periodic(const Duration(seconds: 8), (_) => _refresh());
  }

  void _stop() {
    _timer?.cancel();
    _timer = null;
  }

  Future<void> _refresh() async {
    final c = room.communityId, ch = room.channelId;
    if (c == null || ch == null) return;
    try {
      final snap = await client.bridge.voiceParticipants(c, ch);
      final next = ((snap['participants'] as List?) ?? const [])
          .map((p) => '${(p as Map)['user_id']}')
          .toList();
      if (next.join(',') != _participants.join(',')) {
        _participants = next;
        _changed.add(null);
      }
    } catch (_) {
      // Keep the last known list; the next poll retries.
    }
  }

  @override
  List<String> getCurrentParticipants() => _participants;
  @override
  Stream<void> get onParticipantsChanged => _changed.stream;
  @override
  VoipSession? get currentSession => _session;
  @override
  bool get canJoinCall => room.canConnect;
  @override
  Future<VoipSession?> joinCall() async {
    final c = room.communityId, ch = room.channelId;
    if (c == null || ch == null || !room.canConnect) return null;
    final s = PorchVoipSession(client, room, _voice);
    _session = s;
    try {
      await _voice.join(c, ch);
      s.set(VoipState.connected);
    } catch (_) {
      _session = null;
      rethrow;
    }
    _refresh();
    return s;
  }

  @override
  Future<String?> getCallServerUrl() async => null;
  @override
  Future<void> clearAllCallMembershipStatus() async {}
}
