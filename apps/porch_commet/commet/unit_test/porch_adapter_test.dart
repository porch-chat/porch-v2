import 'dart:io';
import 'package:commet/client/porch/porch_client.dart';
import 'package:commet/config/global_config.dart';
import 'package:commet/client/porch/porch_models.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('permissions are explicit and deny moderation', () {
    final p = PorchPermissions(send: true);
    expect(p.canSendMessage, isTrue);
    expect(p.canBan, isFalse);
    expect(p.canKick, isFalse);
    expect(p.canEditTopic, isFalse);
    expect(PorchPermissions(send: false).canSendMessage, isFalse);
  });

  test('peer identity and colour are stable', () {
    final a = PorchPeer('7', 'sam');
    final b = PorchPeer('7', 'sam');
    expect(a.displayName, 'sam');
    expect(a.defaultColor, b.defaultColor);
    expect(porchHashColor('7'), porchHashColor('7'));
  });

  group('single server login', () {
    test('production server is fixed and not a user field', () {
      const override = String.fromEnvironment('PORCH_SERVER');
      expect(GlobalConfig.porchServer,
          override.isEmpty ? 'https://api.porch.chat' : override);
      expect(GlobalConfig.defaultHomeserver, GlobalConfig.porchServer);
      expect(PorchClient.parseServer(GlobalConfig.porchServer).toString(),
          'https://api.porch.chat');
    });

    test('dev override parsing keeps local fixtures on http', () {
      expect(PorchClient.parseServer('127.0.0.1:8080').toString(),
          'http://127.0.0.1:8080');
      expect(PorchClient.parseServer('http://localhost:9000').toString(),
          'http://localhost:9000');
      expect(PorchClient.parseServer('porch.example').toString(),
          'https://porch.example');
      expect(PorchClient.parseServer('  '), isNull);
    });

    test('login screen has no homeserver field', () {
      final src = File('lib/ui/pages/login/login_page_view.dart')
          .readAsStringSync()
          .toLowerCase();
      expect(src.contains('homeserver'), isFalse);
      expect(src.contains('promptusername'), isTrue);
      expect(src.contains('promptpassword'), isTrue);
    });
  });
}
