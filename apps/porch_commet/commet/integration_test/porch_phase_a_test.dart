// Phase A integration test: real Commet widgets + real Rust bridge against a
// disposable local Porch server (scripts/commet-fixture-draft.py). No acoustic
// verification: media (LiveKit) is not exercised, only the real control plane.
//
// Run: PORCH_FIXTURE=/tmp/porch-commet-fixture.json flutter test \
//   integration_test/porch_phase_a_test.dart -d linux --dart-define=PORCH_SERVER=<url> \
//   --dart-define=PORCH_FIXTURE=/tmp/porch-commet-fixture.json
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;
import 'package:flutter/rendering.dart';

import 'package:commet/client/attachment.dart';
import 'package:commet/client/porch/porch_client.dart';
import 'package:commet/client/porch/porch_files.dart';
import 'package:commet/client/porch/porch_room.dart';
import 'package:commet/client/porch/porch_space.dart';
import 'package:commet/ui/molecules/file_pick_seam.dart';
import 'package:commet/ui/molecules/message_input.dart';
import 'package:commet/ui/pages/login/login_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:integration_test/integration_test.dart';
import 'package:porch_flutter/bridge.dart';

import 'package:commet/main.dart';
import 'package:commet/utils/emoji/unicode_emoji.dart';
import 'package:intl/intl.dart';
import 'package:media_kit/media_kit.dart';
import 'package:tiamat/tiamat.dart' as tiamat;
import 'package:commet/generated/l10n.dart';
import 'extensions/common_flows.dart';
import 'extensions/wait_for.dart';

const fixturePath = String.fromEnvironment('PORCH_FIXTURE');

Future<void> settle(WidgetTester t, [int ms = 1500]) async {
  for (var i = 0; i < ms ~/ 100; i++) {
    await t.pump(const Duration(milliseconds: 100));
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
}

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  binding.framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.fullyLive;
  final fixture = jsonDecode(File(fixturePath).readAsStringSync()) as Map;
  final password = fixture['password'] as String;

  testWidgets('Phase A: real widgets and real control plane', (tester) async {
    // setupApp() uses main.dart's initGuiRequirements, whose Future.wait over
    // intl's SynchronousFuture throws under the test binding; replicate it.
    await tester.clearUserData();
    // The headless box has no desktop notification daemon; its D-Bus failure is
    // unrelated to Porch, so ignore only that one error from the init zone.
    final initDone = Completer<void>();
    runZonedGuarded(() async {
      await initNecessary();
      initDone.complete();
    }, (e, s) {
      if (!'$e'.contains('org.freedesktop.Notifications') && !'$e'.contains('DBus')) {
        if (!initDone.isCompleted) initDone.completeError(e, s);
      }
    });
    await initDone.future;
    MediaKit.ensureInitialized();
    await T.load(const Locale('en'));
    await UnicodeEmojis.load();
    tiamat.getAppScale = () => preferences.appScale.value;
    Intl.defaultLocale = 'en';
    final app = App(clientManager: clientManager!);
    final prevOnError = FlutterError.onError;
    FlutterError.onError = (d) {
      // Upstream Commet layout asserts (Expanded inside Flexible), debug-only, not Porch.
      if (d.toString().contains('Competing ParentDataWidgets')) return;
      prevOnError?.call(d);
    };
    tester.view.physicalSize = const Size(1920, 1080);
    tester.view.devicePixelRatio = 1.0;
    await tester.pumpWidget(app);
    await tester.waitFor(() => find.byType(LoginPage).evaluate().isNotEmpty);

    // Login screen has username + password only.
    await tester.waitFor(() => find.byType(TextField).evaluate().length >= 2);
    final inputs = find.byType(TextField);
    expect(inputs, findsNWidgets(2));
    await tester.enterText(inputs.at(0), 'sam');
    await tester.enterText(inputs.at(1), password);
    await tester.tap(find.widgetWithText(ElevatedButton, 'Login'));
    await tester.waitFor(() => app.clientManager.isLoggedIn(),
        timeout: const Duration(seconds: 20));
    await settle(tester, 2500);
    final client = app.clientManager.clients.first as PorchClient;

    // ---- genuine membership + presence -----------------------------------
    final space = client.spaces.whereType<PorchSpace>().firstWhere(
        (s) => s.displayName == 'Friends');
    final general = client.rooms
        .whereType<PorchRoom>()
        .firstWhere((r) => r.displayName == 'general');
    final names = general.membersList().map((m) => m.displayName).toSet();
    expect(names, containsAll(<String>['sam', 'maya']));
    expect(names.length, 2, reason: 'members come from the server, nothing invented');

    // ---- admin permissions: affordances present ------------------------------
    expect(space.permissions.canInviteUser, isTrue);
    expect(general.permissions.canSendMessage, isTrue);
    expect(general.canAttach, isTrue);

    // ---- real chat view, select channel ---------------------------------------
    app.clientManager.clients; // keep reference
    final texts = find.byType(Text).evaluate().map((e) => (e.widget as Text).data).whereType<String>().toList();
    print('VISIBLE TEXTS: $texts');
    await tester.tap(find.text('F').first);
    await settle(tester, 2000);
    final t2 = find.byType(Text).evaluate().map((e) => (e.widget as Text).data).whereType<String>().toList();
    print('VISIBLE TEXTS2: '+t2.toString());
    await tester.tap(find.text('general').first);
    await settle(tester, 3000);
    expect(find.textContaining('Send a message'), findsWidgets);

    // ---- upload through the injected picker seam into the REAL upload path ----
    final dir = Directory.systemTemp.createTempSync('porch-a-');
    final png = img.encodePng(img.Image(width: 640, height: 400)
      ..clear(img.ColorRgb8(240, 140, 40)));
    final bytes = Uint8List.fromList(png);
    File('${dir.path}/sunset.png').writeAsBytesSync(bytes);
    var pickerCalls = 0;
    filePickOverride = () async {
      pickerCalls++;
      return [PendingFileAttachment(name: 'sunset.png', path: '${dir.path}/sunset.png', size: bytes.length)];
    };
    // Open the composer attachment menu with the real "+" button.
    final plus = find.descendant(of: find.byType(MessageInput), matching: find.byIcon(Icons.add));
    expect(plus, findsOneWidget);
    await tester.tap(plus);
    await settle(tester, 1500);
    // Desktop has a single source so it opens directly; otherwise pick "File".
    if (pickerCalls == 0) {
      await tester.tap(find.text('File'));
      await settle(tester, 1500);
    }
    expect(pickerCalls, 1, reason: 'the real attach menu invoked the picker seam');
    filePickOverride = null;

    // Send with the real send button (message text + queued attachment).
    await tester.tap(find.text('Add File'));
    await settle(tester, 1500);
    await tester.enterText(find.descendant(of: find.byType(MessageInput), matching: find.byType(TextField)).first, 'integration upload');
    await settle(tester, 500);
    print('PRE-SEND texts: '+find.byType(Text).evaluate().map((e) => (e.widget as Text).data).whereType<String>().skip(14).toList().toString());
    await tester.tap(find.descendant(of: find.byType(MessageInput), matching: find.byIcon(Icons.send)).first);
    await settle(tester, 6000);

    // The server now has the message with an attachment and an image preview.
    print('EVENTS: '+general.porchTimeline!.events.map((e)=>e.runtimeType.toString()+':'+((e as dynamic).attachmentList?.length).toString()+':'+((e as dynamic).status).toString()).toList().toString()+' sendIcons='+find.byIcon(Icons.send).evaluate().length.toString());
    final event = general.porchTimeline!.events
        .whereType<dynamic>()
        .where((e) => (e as dynamic).attachmentList?.isNotEmpty == true)
        .toList();
    expect(event, isNotEmpty, reason: 'message with attachment arrived from server');
    final att = (event.first as dynamic).attachments.first;
    expect(att, isA<ImageAttachment>());
    final provider = (att as ImageAttachment).image;
    expect(provider, isA<PorchPreviewImage>());
    final preview = await (provider as PorchPreviewImage).bridge.attachmentPreview(
        provider.conversation, provider.message, provider.attachment);
    expect(preview.length, greaterThan(100));
    expect(preview.sublist(0, 4), [0x89, 0x50, 0x4E, 0x47], reason: 'server preview is a real PNG');
    await settle(tester, 1500);
    await settle(tester, 3000);
    print('IMG: image='+find.byType(Image).evaluate().length.toString()+' raw='+find.byType(RawImage).evaluate().length.toString());
    expect(find.byType(RawImage), findsWidgets, reason: 'preview rendered in the chat');
    await tester.runAsync(() async {
      final layer = tester.binding.renderViews.first.debugLayer as OffsetLayer;
      final image = await layer.toImage(const Rect.fromLTWH(0, 0, 1920, 1080));
      final data = await image.toByteData(format: ui.ImageByteFormat.png);
      File('/home/kit/porch-v2/docs/evidence/commet-phase-a-chat.png')
          .writeAsBytesSync(data!.buffer.asUint8List());
    });
    // ---- invite create + join through the real bridge -----------------------
    final bridge = client.bridge;
    final communityId = general.communityId!;
    print('STEP final code = await bridge.createCommunit');
    final code = await bridge.createCommunityInvite(communityId, 1, 600);
    expect(code, isNotEmpty);
    // Joining as an existing member must be rejected by the real server.
    print('STEP await expectLater(bridge.joinCommunity');
    await expectLater(bridge.joinCommunity(code), throwsA(anything));

    // ---- voice: REAL ticket, lease, flags (control plane only) ------------
    final voice = client.rooms.whereType<PorchRoom>().firstWhere((r) => r.isVoice);
    final ch = voice.channelId!;
    print('STEP final ticket = await bridge.joinVoice');
    final ticket = await bridge.joinVoice(communityId, ch);
    expect(ticket['transport'], isNotNull, reason: 'real server ticket with transport');
    expect(ticket['can_speak'], isTrue);
    print('STEP var snap = await bridge.voiceParticipant');
    var snap = await bridge.voiceParticipants(communityId, ch);
    expect((snap['participants'] as List).length, 1, reason: 'real lease registered');
    await bridge.voiceFlags(true, false);
    snap = await bridge.voiceParticipants(communityId, ch);
    expect(((snap['participants'] as List).first as Map)['flags']['muted'], isTrue);
    await bridge.voiceFlags(true, true);
    snap = await bridge.voiceParticipants(communityId, ch);
    expect(((snap['participants'] as List).first as Map)['flags']['deafened'], isTrue);
    await bridge.leaveVoice();
    snap = await bridge.voiceParticipants(communityId, ch);
    expect((snap['participants'] as List), isEmpty, reason: 'lease cleared on leave');

    // ---- voice channel selection in the real UI -------------------------------
    await tester.tap(find.text('hangout').first);
    await settle(tester, 3000);
    expect(find.text('Join'), findsWidgets);
    // Pause for the evidence screenshot taken by the harness.
    await tester.runAsync(() async {
      final layer = tester.binding.renderViews.first.debugLayer as OffsetLayer;
      final image = await layer.toImage(const Rect.fromLTWH(0, 0, 1920, 1080));
      final data = await image.toByteData(format: ui.ImageByteFormat.png);
      File('/home/kit/porch-v2/docs/evidence/commet-phase-a.png')
          .writeAsBytesSync(data!.buffer.asUint8List());
    });
    File('/tmp/porch-a-ready').writeAsStringSync('ready');
    await settle(tester, 12000);

    // ---- restricted member: real server permissions ------------------------
    await client.logout();
    final maya = client;
    final res = await maya.signIn('maya', password);
    expect(res, isNotNull);
    for (var i = 0; i < 20 && maya.spaces.isEmpty; i++) {
      await settle(tester, 1000);
    }
    print('MAYA SPACES: '+maya.spaces.map((s)=>s.displayName+'/'+s.runtimeType.toString()).toList().toString()+' rooms='+maya.rooms.map((r)=>r.displayName).toList().toString());
    final mSpace = maya.spaces.whereType<PorchSpace>().firstWhere((s) => s.displayName == 'Friends');
    for (var i = 0; i < 20 && mSpace.rooms.isEmpty; i++) {
      await settle(tester, 1000);
    }
    final mGeneral = mSpace.rooms.whereType<PorchRoom>().firstWhere((r) => r.displayName == 'general');
    // maya is not the owner/admin: management affordances must be hidden
    // exactly when the server says so.
    expect(mGeneral.permissions.canSendMessage, isTrue);
    print('MAYA canInvite=${mSpace.canInvite} send=${mGeneral.canSend}');
    expect(mSpace.permissions.canInviteUser, mSpace.canInvite);
    expect(mSpace.permissions.canEditAnything, isFalse);
    expect(mSpace.permissions.canKick, isFalse);
    expect(mSpace.permissions.canBan, isFalse);
    await maya.logout();
  });
}
