import 'package:commet/client/attachment.dart';
import 'package:commet/client/client.dart';
import 'package:commet/client/components/emoticon/emoticon.dart';
import 'package:commet/client/member.dart';
import 'package:commet/client/peer.dart';
import 'package:commet/client/permissions.dart';
import 'package:commet/client/timeline_events/timeline_event_message.dart';
import 'package:commet/client/timeline_events/timeline_event_feature_reactions.dart';
import 'package:flutter/material.dart';

const int porchSendMessages = 1 << 1;

Color porchHashColor(String key) {
  var h = 0;
  for (final c in key.codeUnits) {
    h = (h * 31 + c) & 0x7fffffff;
  }
  return HSLColor.fromAHSL(1, (h % 360).toDouble(), 0.45, 0.55).toColor();
}

class PorchPeer implements Peer, Member {
  @override
  final String identifier;
  @override
  final String userName;
  PorchPeer(this.identifier, this.userName);
  @override
  String get displayName => userName;
  @override
  String? get detail => null;
  @override
  String? get avatarId => null;
  @override
  ImageProvider? get avatar => null;
  @override
  Color get defaultColor => porchHashColor(identifier);
}

/// Every flag is stated explicitly; upstream defaults are permissive.
class PorchPermissions implements Permissions {
  final bool send;
  PorchPermissions({required this.send});
  @override
  bool get canSendMessage => send;
  @override
  bool get canChangeRoles => false;
  @override
  bool get canBan => false;
  @override
  bool get canKick => false;
  @override
  bool get canEditName => false;
  @override
  bool get canEditAvatar => false;
  @override
  bool get canEditTopic => false;
  @override
  bool get canEditAnything => false;
  @override
  bool get canEditAppearance => false;
  @override
  bool get canEnableE2EE => false;
  @override
  bool get canChangeVisibility => false;
  @override
  bool get canEditRoomSecurity => false;
  @override
  bool get canChangeNotificationSettings => false;
  @override
  bool get canUserEditMessages => false;
  @override
  bool get canDeleteOtherUserMessages => false;
  @override
  bool get canEditRoomEmoticons => false;
  @override
  bool get canEditChildren => false;
  @override
  bool get canInviteUser => false;
  @override
  bool get canMentionRoom => false;
}

class PorchMessageEvent extends TimelineEventMessage
    implements TimelineEventFeatureReactions {
  final String nonce;
  int? messageId;
  int revision;
  String senderIdValue;
  String text;
  DateTime time;
  TimelineEventStatus statusValue;
  bool removed;

  PorchMessageEvent({
    required this.nonce,
    required this.messageId,
    required this.revision,
    required this.senderIdValue,
    required this.text,
    required this.time,
    required this.statusValue,
    this.removed = false,
  });

  @override
  String get eventId => messageId == null ? 'p$nonce' : 'm$messageId';
  @override
  String get senderId => senderIdValue;
  @override
  TimelineEventStatus get status => statusValue;
  @override
  String get plainTextBody => removed ? '' : text;
  @override
  String getPlaintextBody(timeline) => plainTextBody;
  @override
  DateTime get originServerTs => time;
  @override
  String get source => '';
  @override
  bool get mentionsRoom => false;
  @override
  List<String> get mentions => const [];
  @override
  bool get editable => false;
  @override
  String? get body => plainTextBody;
  @override
  String? get bodyFormat => null;
  @override
  String? get formattedBody => null;
  @override
  List<Attachment>? get attachments => null;
  @override
  Widget? buildFormattedContent({timeline}) =>
      removed ? null : SelectableText(text);
  @override
  bool isEdited(timeline) => false;
  @override
  List<Uri>? getLinks({timeline}) => null;
  @override
  bool hasReactions(timeline) => false;
  @override
  Map<Emoticon, Set<String>> getReactions(timeline) => const {};
}
