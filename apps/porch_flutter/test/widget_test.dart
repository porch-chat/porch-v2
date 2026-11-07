import 'dart:typed_data';
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:porch_flutter/bridge.dart';
import 'package:porch_flutter/main.dart';

class FakeBridge implements PorchBridge {
  final stream = StreamController<Json>.broadcast();
  bool failSend = false;
  final sent = <String>[];
  final selected = <int>[];
  final credentials = <String>[];
  @override
  Stream<Json> snapshots() => stream.stream;
  @override
  Future<void> login(
    String origin,
    String username,
    String password,
    String cache,
  ) async {
    credentials.addAll([origin, username, password]);
    stream.add(fixture());
  }

  @override
  Future<void> logout() async => stream.add({'user': null});
  @override
  Future<void> select(int id) async {
    selected.add(id);
    stream.add(fixture(selected: id));
  }

  @override
  Future<void> send(int id, String text) async {
    if (failSend) throw StateError('offline');
    sent.add('$id/$text');
  }

  @override
  Future<Json> joinVoice(int community, int channel) async =>
      throw UnimplementedError();
  @override
  Future<void> leaveVoice() async {}
  @override
  Future<void> voiceFlags(bool muted, bool deafened) async {}
  @override
  Future<void> sendFiles(int id, List<String> paths, String text) async {}
  @override
  Future<Uint8List> attachmentPreview(int c, int m, String a) async => Uint8List(0);
  @override
  Future<void> downloadAttachment(int c, int m, String a, String d) async {}
  @override
  Future<String> createCommunityInvite(int c, int u, int e) async => 'code';
  @override
  Future<void> joinCommunity(String code) async {}
  @override
  Future<Json> voiceParticipants(int c, int ch) async => {'participants': []};
}

Json fixture({int selected = 10}) => {
  'user': {'id': 1, 'username': 'sam'},
  'connection': 'Online',
  'communities': [
    {
      'community': {'id': 2, 'name': 'Friends'},
      'channels': [
        {'id': 30, 'name': 'hangout', 'kind': 'voice'},
      ],
      'usernames': {'2': 'maya'},
    },
  ],
  'heads': [
    {'id': 10, 'name': 'general', 'community_id': 2},
    {'id': 11, 'name': 'maya', 'community_id': null},
  ],
  'selected': selected,
  'messages': [
    {
      'id': 1,
      'author_id': 2,
      'content': {'text': 'From the shared engine'},
      'created_at': '2026-11-06T12:00:00Z',
      'attachments': [],
    },
  ],
  'pending': [],
};

void main() {
  late FakeBridge bridge;
  setUp(() => bridge = FakeBridge());
  tearDown(() => bridge.stream.close());

  Future<void> open(WidgetTester tester, {bool signedIn = true}) async {
    tester.view.physicalSize = const Size(1920, 1080);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(Porch(bridge: bridge, cacheRoot: '/test-cache'));
    if (signedIn) bridge.stream.add(fixture());
    await tester.pumpAndSettle();
  }

  testWidgets('sign in forwards credentials then clears password', (
    tester,
  ) async {
    await open(tester, signedIn: false);
    expect(find.text('https://api.porch.chat'), findsOneWidget);
    await tester.enterText(find.widgetWithText(TextField, 'Username'), 'sam');
    await tester.enterText(
      find.widgetWithText(TextField, 'Password'),
      'secret',
    );
    await tester.tap(find.text('Sign in'));
    await tester.pumpAndSettle();
    expect(bridge.credentials, ['https://api.porch.chat', 'sam', 'secret']);
    expect(find.text('From the shared engine'), findsOneWidget);
    expect(find.text('secret'), findsNothing);
  });

  testWidgets('real snapshot list, select DM and durable enqueue', (
    tester,
  ) async {
    await open(tester);
    expect(find.text('From the shared engine'), findsOneWidget);
    expect(find.text('maya'), findsWidgets);
    await tester.tap(find.text('maya').first);
    await tester.pumpAndSettle();
    expect(bridge.selected, [11]);
    await tester.enterText(find.byKey(const Key('composer')), 'Hello');
    await tester.tap(find.byTooltip('Send message'));
    await tester.pumpAndSettle();
    expect(bridge.sent, ['11/Hello']);
    expect(
      tester
          .widget<TextField>(find.byKey(const Key('composer')))
          .controller!
          .text,
      '',
    );
  });

  testWidgets('enqueue failure keeps draft and uses sanitized error', (
    tester,
  ) async {
    await open(tester);
    bridge.failSend = true;
    await tester.enterText(
      find.byKey(const Key('composer')),
      'Keep this draft',
    );
    await tester.tap(find.byTooltip('Send message'));
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<TextField>(find.byKey(const Key('composer')))
          .controller!
          .text,
      'Keep this draft',
    );
    expect(
      find.text('Message could not be queued. Your draft has been kept.'),
      findsOneWidget,
    );
    expect(find.text('offline'), findsNothing);
  });

  testWidgets('community channels and voice controls are not fabricated', (
    tester,
  ) async {
    await open(tester);
    await tester.tap(find.text('Friends'));
    await tester.pumpAndSettle();
    expect(find.text('hangout'), findsOneWidget);
    expect(find.text('general'), findsWidgets);
    expect(find.text('Voice connected'), findsNothing);
    expect(find.text('Add friend'), findsNothing);
  });

  testWidgets('revocation clears message content and composer', (tester) async {
    await open(tester);
    await tester.enterText(find.byKey(const Key('composer')), 'Secret draft');
    bridge.stream.add({'user': null});
    await tester.pumpAndSettle();
    expect(find.text('From the shared engine'), findsNothing);
    expect(find.text('Secret draft'), findsNothing);
    expect(find.text('Sign in'), findsOneWidget);
  });
}
