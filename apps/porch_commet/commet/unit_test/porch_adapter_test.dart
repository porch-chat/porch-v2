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
}
