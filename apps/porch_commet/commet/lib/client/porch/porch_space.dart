import 'dart:async';
import 'dart:typed_data';
import 'package:commet/client/client.dart';
import 'package:commet/client/components/space_component.dart';
import 'package:commet/client/permissions.dart';
import 'package:commet/client/porch/porch_client.dart';
import 'package:commet/client/porch/porch_models.dart';
import 'package:commet/client/porch/porch_room.dart';
import 'package:commet/client/room_preview.dart';
import 'package:commet/client/space_child.dart';
import 'package:commet/client/room.dart';
import 'package:flutter/material.dart';

class PorchSpace extends Space {
  final PorchClient porch;
  final int communityId;
  String name;
  bool canSend = true;
  final List<PorchRoom> _rooms = [];
  final _update = StreamController<void>.broadcast();
  final _added = StreamController<Room>.broadcast();
  final _removed = StreamController<Room>.broadcast();
  final _roomsChanged = StreamController<void>.broadcast();
  final _childUpdated = StreamController<Room>.broadcast();

  PorchSpace(this.porch, this.communityId, this.name);

  void changed() => _update.add(null);
  void roomUpdated(Room r) => _childUpdated.add(r);

  /// Replaces the channel set and returns which rooms were added/removed.
  void setRooms(List<PorchRoom> next) {
    final ids = next.map((r) => r.identifier).toSet();
    for (final old in List<PorchRoom>.from(_rooms)) {
      if (!ids.contains(old.identifier)) {
        _rooms.remove(old);
        _removed.add(old);
      }
    }
    var order = <PorchRoom>[];
    for (final n in next) {
      final i = _rooms.indexWhere((r) => r.identifier == n.identifier);
      if (i < 0) {
        _rooms.add(n);
        _added.add(n);
        order.add(n);
      } else {
        order.add(_rooms[i]);
      }
    }
    _rooms
      ..clear()
      ..addAll(order);
    _roomsChanged.add(null);
  }

  @override
  String get identifier => 'c$communityId';
  @override
  Client get client => porch;
  @override
  Permissions get permissions => PorchPermissions(send: canSend);
  @override
  String get displayName => name;
  @override
  ImageProvider? get avatar => null;
  @override
  List<Room> get rooms => List.unmodifiable(_rooms);
  @override
  List<Space> get subspaces => const [];
  @override
  List<SpaceChild> get children =>
      _rooms.map((r) => SpaceChildRoom(r)).toList();
  @override
  Future<void> setChildrenOrder(List<SpaceChild> children,
          {Function(double?)? onProgressChanged}) async =>
      throw UnsupportedError('Reordering is not supported');
  @override
  bool get isTopLevel => true;
  @override
  String get topic => '';
  @override
  PushRule get pushRule => PushRule.notify;
  @override
  RoomVisibility get visibility => RoomVisibilityPrivate();
  @override
  Color get color => porchHashColor(identifier);
  @override
  String get developerInfo => identifier;
  @override
  Stream<void> get onUpdate => _update.stream;
  @override
  Stream<Room> get onChildRoomUpdated => _childUpdated.stream;
  @override
  Stream<Room> get onRoomAdded => _added.stream;
  @override
  Stream<Room> get onRoomRemoved => _removed.stream;
  @override
  Stream<void> get onChildRoomsUpdated => _roomsChanged.stream;
  @override
  Stream<RoomPreview> get onChildRoomPreviewAdded => const Stream.empty();
  @override
  Stream<RoomPreview> get onChildRoomPreviewRemoved => const Stream.empty();
  @override
  Stream<Space> get onChildSpaceAdded => const Stream.empty();
  @override
  Stream<Space> get onChildSpaceRemoved => const Stream.empty();
  @override
  Stream<void> get onChildRoomPreviewsUpdated => const Stream.empty();
  @override
  List<RoomPreview> get childPreviews => const [];
  @override
  bool get fullyLoaded => true;
  @override
  bool containsRoom(String identifier) =>
      _rooms.any((r) => r.identifier == identifier);
  @override
  Future<List<RoomPreview>> fetchChildren() async => const [];
  @override
  Future<Room> createRoom(String name, CreateRoomArgs args) async =>
      throw UnsupportedError('Creating channels is not supported yet');
  @override
  Future<void> setSpaceChildRoom(Room room) async =>
      throw UnsupportedError('Not supported');
  @override
  Future<void> setSpaceChildSpace(Space room) async =>
      throw UnsupportedError('Not supported');
  @override
  Future<void> removeChild(SpaceChild child) async =>
      throw UnsupportedError('Not supported');
  @override
  Future<void> setTopic(String topic) async =>
      throw UnsupportedError('Not supported');
  @override
  Future<void> loadExtra() async {}
  @override
  Future<void> close() async {
    await _update.close();
    await _added.close();
    await _removed.close();
    await _roomsChanged.close();
    await _childUpdated.close();
  }

  @override
  Future<void> setDisplayName(String newName) async =>
      throw UnsupportedError('Not supported');
  @override
  Future<void> changeAvatar(Uint8List bytes, String? mimeType) async =>
      throw UnsupportedError('Not supported');
  @override
  Future<void> setPushRule(PushRule rule) async =>
      throw UnsupportedError('Not supported');
  @override
  T? getComponent<T extends SpaceComponent>() => null;
}
