// Explicit opt-in only. Seeded loopback fixture, never production.
import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:porch_flutter/src/rust/api.dart' as rust;
import 'package:porch_flutter/src/rust/frb_generated.dart';

void main() {
  final env = Platform.environment;
  final origin = env['PORCH_FIXTURE_ORIGIN'];
  test(
    'generated FFI commands and snapshot stream round trip through core',
    () async {
      await RustLib.init(
        externalLibrary: ExternalLibrary.open(env['PORCH_BRIDGE_LIBRARY']!),
      );
      final values = <Map<String, dynamic>>[];
      final subscription = rust.snapshots().listen((value) {
        values.add(jsonDecode(value) as Map<String, dynamic>);
      });
      final temp = await Directory.systemTemp.createTemp('porch-flutter-ffi-');
      try {
        await rust.login(
          origin: origin!,
          username: env['PORCH_FIXTURE_USERNAME']!,
          password: env['PORCH_FIXTURE_PASSWORD']!,
          cacheRoot: temp.path,
        );
        final initial =
            jsonDecode(await rust.snapshot()) as Map<String, dynamic>;
        expect(initial['user']['username'], env['PORCH_FIXTURE_USERNAME']);
        final head = (initial['heads'] as List).firstWhere(
          (h) =>
              h['name'] == 'general' &&
              h['community_id'] == int.parse(env['PORCH_FIXTURE_COMMUNITY']!),
        );
        final id = head['id'] as int;
        await rust.selectConversation(id: id);
        await rust.sendMessage(
          conversation: id,
          text: 'Generated Flutter bridge smoke',
        );
        final deadline = DateTime.now().add(const Duration(seconds: 20));
        while (!values.any(
          (v) => (v['messages'] as List? ?? []).any(
            (m) => m['content']?['text'] == 'Generated Flutter bridge smoke',
          ),
        )) {
          if (DateTime.now().isAfter(deadline)) {
            fail('No confirmed message from native snapshot stream');
          }
          await Future<void>.delayed(const Duration(milliseconds: 100));
        }
        expect(values.any((v) => v['connection'] == 'Online'), isTrue);
        expect(values.last.toString().contains('token'), isFalse);
        await rust.logout();
        expect(jsonDecode(await rust.snapshot())['user'], isNull);
      } finally {
        await subscription.cancel();
        await rust.logout();
        await temp.delete(recursive: true);
      }
    },
    skip: origin == null ? 'requires explicit seeded loopback fixture' : false,
  );
}
