import 'package:commet/client/porch/porch_client.dart';
import 'package:commet/client/porch/porch_space.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:tiamat/tiamat.dart' as tiamat;

/// Creates a single invite code. The server shows it once, so copy it before closing.
class PorchInviteDialog extends StatefulWidget {
  final PorchClient client;
  final PorchSpace space;
  const PorchInviteDialog(this.client, this.space, {super.key});
  @override
  State<PorchInviteDialog> createState() => _PorchInviteDialogState();
}

class _PorchInviteDialogState extends State<PorchInviteDialog> {
  int maxUses = 1;
  int hours = 24;
  String? code;
  String? error;
  bool busy = false;

  Future<void> create() async {
    setState(() {
      busy = true;
      error = null;
    });
    try {
      final c = await widget.client.bridge.createCommunityInvite(
          widget.space.communityId, maxUses, hours * 3600);
      if (mounted) setState(() => code = c);
    } catch (e) {
      if (mounted) setState(() => error = 'Could not create the invite.');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 400,
      child: Column(mainAxisSize: MainAxisSize.min, children: [
        if (code == null) ...[
          Row(children: [
            const Text('Uses'),
            const Spacer(),
            DropdownButton<int>(
                value: maxUses,
                items: [1, 5, 10, 50, 200]
                    .map((e) => DropdownMenuItem(value: e, child: Text('$e')))
                    .toList(),
                onChanged: (v) => setState(() => maxUses = v ?? 1)),
          ]),
          Row(children: [
            const Text('Expires after'),
            const Spacer(),
            DropdownButton<int>(
                value: hours,
                items: const [1, 24, 168, 720]
                    .map((e) => DropdownMenuItem(value: e, child: Text('$e h')))
                    .toList(),
                onChanged: (v) => setState(() => hours = v ?? 24)),
          ]),
          const SizedBox(height: 12),
          tiamat.Button(text: 'Create invite', isLoading: busy, onTap: create),
        ] else ...[
          SelectableText(code!),
          const SizedBox(height: 12),
          tiamat.Button(
              text: 'Copy code',
              onTap: () => Clipboard.setData(ClipboardData(text: code!))),
        ],
        if (error != null) Text(error!),
      ]),
    );
  }
}
