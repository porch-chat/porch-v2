import 'dart:async';
import 'dart:typed_data';
import 'package:commet/client/attachment.dart';
import 'package:commet/client/client.dart';
import 'package:commet/client/components/emoticon/emoticon.dart';
import 'package:commet/client/components/room_component.dart';
import 'package:commet/client/member.dart';
import 'package:commet/client/permissions.dart';
import 'package:commet/client/porch/porch_client.dart';
import 'package:commet/client/porch/porch_models.dart';
import 'package:commet/client/porch/porch_timeline.dart';
import 'package:commet/client/role.dart';
import 'package:commet/client/timeline_events/timeline_event.dart';
import 'package:flutter/material.dart';

class PorchRoom extends Room {
  final PorchClient porch;
  @override
  final String identifier;
  int? headId;
  final int? communityId;
  final int? channelId;
  final bool isVoice;
  String name;
  List<String> participants;
  int unread = 0, mentions = 0;
  int lastMessageId = 0;
  bool canSend = true;
  PorchTimeline? _timeline;
  final _update = StreamController<void>.broadcast();
  String? lastReadValue;

  PorchRoom(this.porch,
      {required this.identifier,
      required this.name,
      this.headId,
      this.communityId,
      this.channelId,
      this.isVoice = false,
      this.participants = const []});

  void changed() => _update.add(null);

  @override
  Client get client => porch;
  @override
  Timeline? get timeline => _timeline;
  PorchTimeline? get porchTimeline => _timeline;
  @override
  ImageProvider? get avatar => null;
  @override
  String? get avatarId => null;
  @override
  Iterable<String> get memberIds => communityId == null
      ? participants
      : porch.memberIdsOf(communityId!);
  @override
  String get displayName => name;
  @override
  String? get topic => null;
  @override
  Permissions get permissions =>
      PorchPermissions(send: canSend && !isVoice && headId != null);
  @override
  bool get isE2EE => false;
  @override
  bool get isSpecialRoomType => isVoice;
  @override
  Future<void> setAsFavorite(bool favorite) async =>
      throw UnsupportedError('Favorites are not supported');
  @override
  bool get isFavorite => false;
  @override
  bool get shouldPreviewMedia => false;
  @override
  Color get defaultColor => porchHashColor(identifier);
  @override
  Stream<void> get onUpdate => _update.stream;
  @override
  PushRule get pushRule => PushRule.notify;
  @override
  DateTime get lastEventTimestamp =>
      (_timeline?.events.isNotEmpty ?? false)
          ? _timeline!.events.first.originServerTs
          : DateTime.fromMillisecondsSinceEpoch(lastMessageId);
  @override
  String get developerInfo => identifier;
  @override
  int get notificationCount => unread;
  @override
  int get highlightedNotificationCount => mentions;
  @override
  RoomVisibility get visibility => RoomVisibilityPrivate();

  @override
  Future<TimelineEvent?> sendMessage({
    String? message,
    TimelineEvent? inReplyTo,
    TimelineEvent? replaceEvent,
    List<ProcessedAttachment> processedAttachments = const [],
  }) async {
    if (headId == null) throw UnsupportedError('This channel has no text');
    if (inReplyTo != null || replaceEvent != null) {
      throw UnsupportedError('Replies and edits are not supported yet');
    }
    if (processedAttachments.isNotEmpty) {
      throw UnsupportedError('Attachments are not supported yet');
    }
    final text = message?.trim() ?? '';
    if (text.isEmpty) return null;
    await porch.bridge.send(headId!, text);
    return null;
  }

  @override
  Future<void> cancelSend(TimelineEvent event) async =>
      throw UnsupportedError('Cancelling a queued send is not supported');
  @override
  Future<void> retrySend(TimelineEvent event) async =>
      throw UnsupportedError('Retry is automatic');
  @override
  Future<TimelineEvent?> addReaction(
          TimelineEvent reactingTo, Emoticon reaction) async =>
      throw UnsupportedError('Reactions are not supported yet');
  @override
  Future<void> removeReaction(
          TimelineEvent reactingTo, Emoticon reaction) async =>
      throw UnsupportedError('Reactions are not supported yet');
  @override
  Future<List<ProcessedAttachment>> processAttachments(
          List<PendingFileAttachment> attachments) async {
    if (attachments.isEmpty) return const [];
    throw UnsupportedError('Attachments are not supported yet');
  }
  @override
  List<Member> membersList() => porch.membersOf(this);
  @override
  Future<List<Member>> fetchMembersList({bool cache = false}) async =>
      membersList();
  @override
  List<(Member, Role)> importantMembers() => const [];
  @override
  Role getMemberRole(String identifier) => const PorchRole();
  @override
  bool get isMembersListComplete => true;
  @override
  Future<void> setDisplayName(String newName) async =>
      throw UnsupportedError('Renaming is not supported yet');
  @override
  Future<void> setRoomAvatar(Uint8List bytes, String? mimeType) async =>
      throw UnsupportedError('Avatars are not supported yet');
  @override
  Future<void> setPushRule(PushRule rule) async =>
      throw UnsupportedError('Notification settings are not supported yet');
  @override
  Future<void> setVisibility(RoomVisibility visibility) async =>
      throw UnsupportedError('Visibility is not supported');
  @override
  Color getColorOfUser(String userId) => porchHashColor(userId);

  @override
  Future<Timeline> getTimeline({String? contextEventId}) async {
    final id = headId;
    if (id == null) throw UnsupportedError('Voice channels have no timeline');
    _timeline ??= PorchTimeline(porch, this, id, porch.userName);
    await porch.bridge.select(id);
    return _timeline!;
  }

  @override
  Future<void> enableE2EE() async =>
      throw UnsupportedError('Encryption is not supported');
  @override
  Future<void> close() async {
    await _update.close();
  }

  @override
  bool shouldNotify(TimelineEvent event) =>
      event.senderId != porch.selfId && event.status.isSynced;
  @override
  TimelineEvent? get lastEvent => _timeline?.events.firstOrNull;
  @override
  TimelineEvent? get lastMessage => lastEvent;
  @override
  T? getComponent<T extends RoomComponent>() => porch.roomComponent<T>(this);
  @override
  List<T> getAllComponents<T extends RoomComponent<Client, Room>>() {
    final c = getComponent<T>();
    return c == null ? <T>[] : <T>[c];
  }

  @override
  Future<ImageProvider?> getShortcutImage() async => null;
  @override
  Future<TimelineEvent?> getEvent(String eventId) async =>
      _timeline?.tryGetEvent(eventId);
  @override
  Future<void> kickUser(String id) async =>
      throw UnsupportedError('Not supported');
  @override
  Future<void> banUser(String id) async =>
      throw UnsupportedError('Not supported');
  @override
  Member getMemberOrFallback(String id) =>
      porch.peerOf(id);
  @override
  Member? getMember(String id) => porch.peerOf(id);
  @override
  Future<Member> fetchMember(String id) async => porch.peerOf(id);
  @override
  List<Role> get availableRoles => const [];
  @override
  Future<void> setMemberRole(String id, Role role) async =>
      throw UnsupportedError('Not supported');
  @override
  Future<void> setTopic(String topic) async =>
      throw UnsupportedError('Not supported');
  @override
  Future<void> markAsRead() async {}
  @override
  String? get lastRead => lastReadValue;
}

/// Porch has no per-room role display yet; every member shows as a plain member.
class PorchRole implements Role {
  const PorchRole();
  @override
  String get name => 'Member';
  @override
  IconData get icon => Icons.person;
}
