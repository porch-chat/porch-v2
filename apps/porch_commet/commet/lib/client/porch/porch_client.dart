import 'dart:async';
import 'dart:typed_data';
import 'package:commet/client/auth.dart';
import 'package:commet/client/client.dart';
import 'package:commet/client/components/component.dart';
import 'package:commet/client/components/direct_messages/direct_message_component.dart';
import 'package:commet/client/components/room_component.dart';
import 'package:commet/client/components/sidebar_component/sidebar_entries_component.dart';
import 'package:commet/client/member.dart';
import 'package:commet/client/porch/porch_models.dart';
import 'package:commet/client/porch/porch_profile.dart';
import 'package:commet/client/components/profile/profile_component.dart';
import 'package:commet/client/porch/porch_room.dart';
import 'package:commet/client/porch/porch_space.dart';
import 'package:commet/client/room_preview.dart';
import 'package:commet/utils/notifying_list.dart';
import 'package:commet/utils/notifying_list_filter.dart';
import 'package:commet/utils/stored_stream_controller.dart';
import 'package:commet/client/timeline_events/timeline_event.dart';
import 'package:flutter/material.dart';
import 'package:porch_flutter/bridge.dart';

typedef Json = Map<String, dynamic>;

class PorchPasswordLogin implements PasswordLoginFlow {
  @override
  String? username;
  @override
  String? password;
  @override
  Future<LoginResult> submit(Client client) =>
      (client as PorchClient).signIn(username ?? '', password ?? '');
}

class PorchClient extends Client
    implements DirectMessagesComponent<PorchClient> {
  final PorchBridge bridge;
  late final PorchProfileComponent _profile = PorchProfileComponent(this);
  final String cacheDir;
  String origin = 'https://api.porch.chat';
  StreamSubscription<Json>? _sub;

  PorchClient(this.bridge, this.cacheDir) {
    favoriteRooms = NotifyingListFilter(rooms, where: (_) => false);
  }

  final _rooms = NotifyingList<Room>.empty(growable: true);
  final _dms = NotifyingList<Room>.empty(growable: true);
  final _spaces = <PorchSpace>[];
  final _peers = <String, PorchPeer>{};
  final _usernames = <String, String>{};
  final _members = <int, List<String>>{};
  final _roomMap = <String, PorchRoom>{};
  final _onRoomAdded = StreamController<Room>.broadcast();
  final _onSpaceAdded = StreamController<Space>.broadcast();
  final _onRoomRemoved = StreamController<Room>.broadcast();
  final _onSpaceRemoved = StreamController<Space>.broadcast();
  final _onPeerAdded = StreamController<Peer>.broadcast();
  final _onSync = StreamController<void>.broadcast();
  final _onTimeline = StreamController<(Room, TimelineEvent)>.broadcast();
  final _status = StoredStreamController<ClientConnectionStatusUpdate>(
      ClientConnectionStatusUpdate(ClientConnectionStatus.unknown));
  Json _last = {};
  bool _loggedIn = false;
  Completer<LoginResult>? _login;
  String? lastError;
  int? selectedId;

  String? get selfId {
    final u = _last['user'];
    return u == null ? null : '${u['id']}';
  }

  String userName(String id) => _usernames[id] ?? id;
  Iterable<String> memberIdsOf(int community) =>
      _members[community] ?? const [];
  PorchPeer peerOf(String id) =>
      _peers.putIfAbsent(id, () => PorchPeer(id, userName(id)));
  List<Member> membersOf(PorchRoom r) =>
      r.memberIds.map((e) => peerOf(e) as Member).toList();

  @override
  String get identifier => 'porch';
  @override
  bool get supportsE2EE => false;
  @override
  int? get maxFileSize => null;
  @override
  List<Room> get singleRooms => List.unmodifiable(_dms);
  @override
  NotifyingList<Room> get rooms => _rooms;
  @override
  List<Space> get spaces => List.unmodifiable(_spaces);
  @override
  List<Peer> get peers => _peers.values.toList();
  @override
  Stream<Room> get onRoomAdded => _onRoomAdded.stream;
  @override
  Stream<Space> get onSpaceAdded => _onSpaceAdded.stream;
  @override
  Stream<Room> get onRoomRemoved => _onRoomRemoved.stream;
  @override
  Stream<Space> get onSpaceRemoved => _onSpaceRemoved.stream;
  @override
  Stream<Peer> get onPeerAdded => _onPeerAdded.stream;
  @override
  Stream<void> get onSync => _onSync.stream;
  @override
  Stream<(Room, TimelineEvent)> get onTimelineEvent => _onTimeline.stream;
  @override
  late NotifyingListFilter<Room> favoriteRooms;
  @override
  StoredStreamController<ClientConnectionStatusUpdate>
      get connectionStatusChanged => _status;

  @override
  Future<void> init(bool loadingFromCache,
      {bool isBackgroundService = false}) async {
    _sub ??= bridge.snapshots().listen(_onSnapshot);
  }

  /// Normalizes a server setting to a base Uri. Bare hosts get https, except
  /// localhost / loopback which get http. Returns null if unparseable.
  static Uri? parseServer(String input) {
    final host = input.trim().replaceFirst(RegExp(r'^https?://'), '');
    if (host.isEmpty) return null;
    final local = host.startsWith('localhost') || host.startsWith('127.0.0.1');
    try {
      return Uri.parse(local ? 'http://$host' : 'https://$host');
    } on FormatException {
      return null;
    }
  }

  @override
  Future<(bool, List<LoginFlow>?)> setHomeserver(Uri uri) async {
    origin = uri.origin;
    return (true, <LoginFlow>[PorchPasswordLogin()]);
  }

  late final _sidebar = PorchSidebar(this);

  static String _loginMessage(String raw) {
    if (raw.contains('401')) return 'Wrong username or password.';
    if (raw.contains('429')) return 'Too many attempts. Try again later.';
    if (raw.contains('Connect') || raw.contains('error sending request')) {
      return 'Could not reach the server.';
    }
    return 'Sign in failed.';
  }

  Future<LoginResult> signIn(String user, String pass) async {
    await init(false);
    _login = Completer<LoginResult>();
    lastError = null;
    try {
      await bridge.login(origin, user, pass, cacheDir);
    } catch (e) {
      _login = null;
      return LoginResultError(_loginMessage('$e'));
    }
    return _login!.future.timeout(const Duration(seconds: 20),
        onTimeout: () => LoginResultError('Sign in timed out'));
  }

  @override
  Future<LoginResult> executeLoginFlow(LoginFlow flow) => flow.submit(this);
  @override
  Future<bool> hasServerDisabledEncryption() async => true;

  @override
  Future<void> logout() async {
    await bridge.logout();
    _loggedIn = false;
    for (final s in List<PorchSpace>.from(_spaces)) {
      _spaces.remove(s);
      _onSpaceRemoved.add(s);
    }
    for (final r in List<Room>.from(_rooms)) {
      _rooms.remove(r);
      _onRoomRemoved.add(r);
    }
    _dms.clear();
    _roomMap.clear();
    self = null;
  }

  @override
  bool isLoggedIn() => _loggedIn;
  @override
  bool hasSpace(String identifier) => getSpace(identifier) != null;
  @override
  bool hasRoom(String identifier) => _roomMap.containsKey(identifier);
  @override
  bool hasPeer(String identifier) => _peers.containsKey(identifier);
  @override
  Room? getRoom(String identifier) => _roomMap[identifier];
  @override
  Room? getRoomByAlias(String identifier) => null;
  @override
  Space? getSpace(String identifier) {
    for (final s in _spaces) {
      if (s.identifier == identifier) return s;
    }
    return null;
  }

  Never _no(String what) =>
      throw UnsupportedError('$what is not supported yet');
  @override
  Future<Room> createRoom(CreateRoomArgs args) async => _no('Creating rooms');
  @override
  Future<Space> createSpace(CreateRoomArgs args) async =>
      _no('Creating communities');
  @override
  Future<Space> joinSpace(String address) async => _no('Joining by address');
  @override
  Future<Room> joinRoom(String address) async => _no('Joining by address');
  @override
  Future<Room> joinRoomFromPreview(RoomPreview preview) async => _no('Joining');
  @override
  Future<void> leaveRoom(Room room) async => _no('Leaving');
  @override
  Future<void> leaveSpace(Space space) async => _no('Leaving');
  @override
  Future<RoomPreview?> getSpacePreview(String address) async => null;
  @override
  Future<RoomPreview?> getRoomPreview(String address) async => null;
  @override
  Future<void> setAvatar(Uint8List bytes, String mimeType) async =>
      _no('Avatars');
  @override
  Future<void> setDisplayName(String name) async => _no('Renaming');
  @override
  Future<void> close() async {
    await _sub?.cancel();
    _sub = null;
  }

  @override
  Iterable<Room> getEligibleRoomsForSpace(Space space) => const [];
  @override
  Widget buildDebugInfo() => Text(origin);

  @override
  T? getComponent<T extends Component>() {
    if (this is T) return this as T;
    if (_sidebar is T) return _sidebar as T;
    if (_profile is T) return _profile as T;
    return null;
  }

  @override
  List<T>? getAllComponents<T extends Component>() {
    final c = getComponent<T>();
    return c == null ? <T>[] : <T>[c];
  }

  T? roomComponent<T extends RoomComponent>(PorchRoom room) => null;

  // DirectMessagesComponent
  @override
  PorchClient get client => this;
  @override
  INotifyingList<Room> get directMessageRooms => _dms;
  @override
  INotifyingList<Room> get highlightedRoomsList => _dms;
  @override
  bool isRoomDirectMessage(Room room) => _dms.contains(room);
  @override
  String? getDirectMessagePartnerId(Room room) {
    if (room is! PorchRoom) return null;
    final me = selfId;
    for (final p in room.participants) {
      if (p != me) return p;
    }
    return null;
  }

  @override
  Future<Room?> createDirectMessage(String userId) async =>
      _no('Creating direct messages');

  Future<void> select(PorchRoom room) async {
    selectedId = room.headId;
  }

  void _onSnapshot(Json s) {
    _last = s;
    final err = s['error'];
    if (err != null) lastError = '$err';
    final user = s['user'] as Json?;
    if (user == null) {
      if (_login != null && !_login!.isCompleted && err != null) {
        _login!.complete(LoginResultError('$err'));
      }
      _status.add(
          ClientConnectionStatusUpdate(ClientConnectionStatus.disconnected));
      return;
    }
    final uid = '${user['id']}';
    _usernames[uid] = user['username'] as String;
    final first = !_loggedIn;
    _loggedIn = true;
    self ??= PorchProfile(uid, user['username'] as String);
    final conn = '${s['connection']}'.toLowerCase();
    _status.add(ClientConnectionStatusUpdate(conn == 'online'
        ? ClientConnectionStatus.connected
        : ClientConnectionStatus.connecting));
    _syncCommunities(s);
    _syncHeads(s);
    final sel = s['selected'];
    for (final r in _roomMap.values) {
      final t = r.porchTimeline;
      if (t != null && r.headId == sel) {
        final before = t.events.length;
        t.apply(((s['messages'] as List?) ?? const []).cast<Json>(),
            ((s['pending'] as List?) ?? const []).cast<Json>(), uid);
        if (t.events.length > before && t.events.isNotEmpty) {
          _onTimeline.add((r, t.events.first));
        }
      }
    }
    _onSync.add(null);
    if (first && _login != null && !_login!.isCompleted) {
      _login!.complete(LoginResultSuccess());
    }
  }

  void _syncCommunities(Json s) {
    final list = ((s['communities'] as List?) ?? const []).cast<Json>();
    final keep = <String>{};
    for (final entry in list) {
      final c = entry['community'] as Json;
      final id = c['id'] as int;
      final names = (entry['usernames'] as Map?)?.cast<String, dynamic>() ?? {};
      names.forEach((k, v) => _usernames[k] = '$v');
      _members[id] = ((entry['members'] as List?) ?? const [])
          .map((m) => '${(m as Json)['user_id']}')
          .toList();
      for (final m in _members[id]!) {
        if (!_peers.containsKey(m)) {
          final p = peerOf(m);
          _onPeerAdded.add(p);
        }
      }
      var space = _spaces
          .cast<PorchSpace?>()
          .firstWhere((x) => x!.communityId == id, orElse: () => null);
      var isNew = false;
      if (space == null) {
        space = PorchSpace(this, id, c['name'] as String);
        _spaces.add(space);
        isNew = true;
      } else if (space.name != c['name']) {
        space.name = c['name'] as String;
        space.changed();
      }
      keep.add(space.identifier);
      space.canSend =
          ((entry['permissions'] as int? ?? 0) & porchSendMessages) != 0;
      final next = <PorchRoom>[];
      for (final ch
          in ((entry['channels'] as List?) ?? const []).cast<Json>()) {
        final rid = 'ch${ch['id']}';
        var room = _roomMap[rid];
        final voice = ch['kind'] == 'voice';
        if (room == null) {
          room = PorchRoom(this,
              identifier: rid,
              name: ch['name'] as String,
              communityId: id,
              channelId: ch['id'] as int,
              isVoice: voice);
          _roomMap[rid] = room;
          _rooms.add(room);
          _onRoomAdded.add(room);
        } else if (room.name != ch['name']) {
          room.name = ch['name'] as String;
          room.changed();
        }
        room.canSend = space.canSend;
        next.add(room);
      }
      space.setRooms(next);
      if (isNew) _onSpaceAdded.add(space);
    }
    for (final sp in List<PorchSpace>.from(_spaces)) {
      if (!keep.contains(sp.identifier)) {
        for (final r in sp.rooms) {
          _roomMap.remove(r.identifier);
          _rooms.remove(r);
          _onRoomRemoved.add(r);
        }
        _spaces.remove(sp);
        _onSpaceRemoved.add(sp);
      }
    }
  }

  void _syncHeads(Json s) {
    final heads = ((s['heads'] as List?) ?? const []).cast<Json>();
    for (final h in heads) {
      final hid = h['id'] as int;
      final cid = h['community_id'];
      PorchRoom? room;
      if (cid == null) {
        final rid = 'dm$hid';
        room = _roomMap[rid];
        final parts = ((h['participants'] as List?) ?? const [])
            .map((e) => '$e')
            .toList();
        final name = parts.where((p) => p != selfId).map(userName).join(', ');
        if (room == null) {
          room = PorchRoom(this,
              identifier: rid,
              name: name.isEmpty
                  ? (h['name'] as String? ?? 'Direct message')
                  : name,
              headId: hid,
              participants: parts);
          _roomMap[rid] = room;
          _rooms.add(room);
          _dms.add(room);
          _onRoomAdded.add(room);
        }
      } else {
        room = _roomMap['ch${h['channel_id']}'];
        if (room == null) continue;
        room.headId = hid;
      }
      _applyHead(room, h);
    }
  }

  void _applyHead(PorchRoom room, Json h) {
    final unread = h['unread_count'] as int? ?? 0;
    final mention = h['mention_count'] as int? ?? 0;
    final last = h['last_message_id'] as int? ?? 0;
    final changed = room.unread != unread ||
        room.mentions != mention ||
        room.lastMessageId != last;
    room.unread = unread;
    room.mentions = mention;
    room.lastMessageId = last;
    if (changed) {
      room.changed();
      for (final sp in _spaces) {
        if (sp.containsRoom(room.identifier)) sp.roomUpdated(room);
      }
    }
  }
}

class PorchProfile implements Profile {
  @override
  final String identifier;
  @override
  final String userName;
  PorchProfile(this.identifier, this.userName);
  @override
  String get displayName => userName;
  @override
  String? get detail => null;
  @override
  ImageProvider? get avatar => null;
  @override
  ImageProvider? get banner => null;
  @override
  Color get defaultColor => porchHashColor(identifier);
  @override
  String get source => 'porch';
}

/// Communities in server order. Folders are a Matrix account-data feature and are unsupported.
class PorchSidebar implements SidebarEntriesComponent<PorchClient> {
  @override
  final PorchClient client;
  PorchSidebar(this.client);

  @override
  List<SidebarEntry> getEntries() {
    final spaces = client.spaces;
    return [
      for (var i = 0; i < spaces.length; i++)
        SpaceSidebarEntry(spaces[i], order: i.toString().padLeft(6, '0'))
    ];
  }

  @override
  String createFolder(Space space) => throw UnsupportedError('Folders');
  @override
  void save() {}
  @override
  addToFolder(Space space, String folderId, int index) {}
  @override
  removeFromFolder(Space space, String folderId) {}
}
