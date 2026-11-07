import 'dart:async';
import 'package:commet/client/attachment.dart';
import 'package:commet/client/client.dart';
import 'package:commet/client/porch/porch_client.dart';
import 'package:commet/client/porch/porch_files.dart';
import 'package:commet/client/porch/porch_models.dart';
import 'package:commet/client/timeline_events/timeline_event.dart';

typedef Json = Map<String, dynamic>;

/// Applies bridge snapshots as targeted add/change/remove notifications.
/// Pending sends are reconciled to confirmed messages by nonce.
class PorchTimeline extends Timeline {
  final int conversationId;
  final String Function(String id) nameOf;
  PorchTimeline(Client client, Room room, this.conversationId, this.nameOf) {
    this.client = client;
    this.room = room;
  }

  PorchMessageEvent? byNonce(String nonce) {
    for (final e in events) {
      if (e is PorchMessageEvent && e.nonce == nonce) return e;
    }
    return null;
  }

  List<Attachment>? _attachments(Json m) {
    final raw = m['attachments'] as List?;
    final c = client;
    if (raw == null || raw.isEmpty || c is! PorchClient) return null;
    return porchAttachments(
        c.bridge, c.cacheDir, conversationId, m['id'] as int, raw);
  }

  void apply(List<Json> messages, List<Json> pending, String selfId) {
    final confirmed = messages
        .where((m) => m['conversation_id'] == conversationId)
        .toList()
      ..sort((a, b) => (b['id'] as int).compareTo(a['id'] as int));
    final desired = <PorchMessageEvent>[];
    final seen = <String>{};
    for (final p in pending.reversed) {
      if (p['conversation_id'] != conversationId) continue;
      final r = p['request'] as Json;
      final nonce = r['nonce'] as String;
      if (confirmed.any((m) => m['nonce'] == nonce)) continue;
      final failed = p['state'] == 'failed';
      desired.add(PorchMessageEvent(
        nonce: nonce,
        messageId: null,
        revision: 0,
        senderIdValue: selfId,
        text: (r['content']['text'] as String?) ?? '',
        time: byNonce(nonce)?.time ?? DateTime.now(),
        statusValue:
            failed ? TimelineEventStatus.error : TimelineEventStatus.sending,
      ));
    }
    for (final m in confirmed) {
      final content = m['content'] as Json?;
      desired.add(PorchMessageEvent(
        nonce: m['nonce'] as String,
        messageId: m['id'] as int,
        revision: m['revision'] as int? ?? 0,
        senderIdValue: '${m['author_id']}',
        text: content?['text'] as String? ?? '',
        time: DateTime.tryParse(m['created_at'] as String? ?? '') ??
            DateTime.now(),
        statusValue: TimelineEventStatus.synced,
        removed: m['deleted'] == true,
        attachmentList: _attachments(m),
      ));
    }
    for (final d in desired) {
      seen.add(d.nonce);
    }
    for (var i = events.length - 1; i >= 0; i--) {
      final e = events[i] as PorchMessageEvent;
      if (!seen.contains(e.nonce)) {
        events.removeAt(i);
        onRemove.add(i);
      }
    }
    for (var i = 0; i < desired.length; i++) {
      final d = desired[i];
      final at = i < events.length ? events[i] as PorchMessageEvent : null;
      if (at != null && at.nonce == d.nonce) {
        if (at.messageId != d.messageId ||
            at.revision != d.revision ||
            at.statusValue != d.statusValue ||
            at.text != d.text ||
            at.removed != d.removed ||
            at.attachmentList?.length != d.attachmentList?.length) {
          at.attachmentList = d.attachmentList;
          at.messageId = d.messageId;
          at.revision = d.revision;
          at.statusValue = d.statusValue;
          at.text = d.text;
          at.time = d.time;
          at.removed = d.removed;
          notifyChanged(i);
        }
        continue;
      }
      final existing =
          events.indexWhere((e) => (e as PorchMessageEvent).nonce == d.nonce);
      if (existing >= 0) {
        events.removeAt(existing);
        onRemove.add(existing);
      }
      insertEvent(i, d);
    }
  }

  @override
  void markAsRead(TimelineEvent event) {}
  @override
  Future<void> loadMoreHistory() async {}
  @override
  Future<void> loadMoreFuture() async {}
  @override
  bool get isLoadingHistory => false;
  @override
  bool get isLoadingFuture => false;
  @override
  bool get canLoadFuture => false;
  @override
  bool get canLoadHistory => false;
  @override
  Stream<void> get onLoadingStatusChanged => const Stream.empty();
  @override
  Future<void> close() async {}
  @override
  Future<TimelineEvent?> fetchEventByIdInternal(String eventId) async =>
      tryGetEvent(eventId);
  @override
  String getDisplayId(TimelineEvent event) => event.eventId;
  @override
  bool canDeleteEvent(TimelineEvent event) => false;
  @override
  void deleteEvent(TimelineEvent event) =>
      throw UnsupportedError('Deleting messages is not supported yet');
  @override
  bool isEventRedacted(TimelineEvent event) =>
      event is PorchMessageEvent && event.removed;
}
