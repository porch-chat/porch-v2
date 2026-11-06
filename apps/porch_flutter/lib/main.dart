import 'dart:async';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_chat_core/flutter_chat_core.dart';
import 'package:flutter_chat_ui/flutter_chat_ui.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:path_provider/path_provider.dart';
import 'bridge.dart';
import 'voice.dart';
import 'src/rust/frb_generated.dart';

const bg = Color(0xff16130f),
    panel = Color(0xff1c1813),
    cream = Color(0xfff3e9d8),
    amber = Color(0xfff2a541),
    muted = Color(0xffa39787),
    line = Color(0xff332c23);

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final executable = File(Platform.resolvedExecutable).parent.path;
  final path = Platform.isWindows
      ? '$executable/porch_flutter.dll'
      : '$executable/lib/libporch_flutter.so';
  await RustLib.init(externalLibrary: ExternalLibrary.open(path));
  final root = await getApplicationSupportDirectory();
  runApp(Porch(bridge: RustPorchBridge(), cacheRoot: '${root.path}/cache'));
}

class Porch extends StatelessWidget {
  final PorchBridge bridge;
  final String cacheRoot;
  const Porch({super.key, required this.bridge, required this.cacheRoot});
  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'Porch',
    debugShowCheckedModeBanner: false,
    theme: ThemeData(
      brightness: Brightness.dark,
      scaffoldBackgroundColor: bg,
      fontFamily: 'Manrope',
      colorScheme: const ColorScheme.dark(
        primary: amber,
        surface: bg,
        onSurface: cream,
      ),
      textTheme: ThemeData.dark().textTheme.apply(
        fontFamily: 'Manrope',
        bodyColor: cream,
        displayColor: cream,
      ),
      dividerColor: line,
    ),
    home: Shell(bridge: bridge, cacheRoot: cacheRoot),
  );
}

class Shell extends StatefulWidget {
  final PorchBridge bridge;
  final String cacheRoot;
  const Shell({super.key, required this.bridge, required this.cacheRoot});
  @override
  State<Shell> createState() => _ShellState();
}

class _ShellState extends State<Shell> with WidgetsBindingObserver {
  final origin = TextEditingController(text: 'https://api.porch.chat');
  final username = TextEditingController();
  final password = TextEditingController();
  final composer = TextEditingController();
  final chat = InMemoryChatController();
  StreamSubscription<Json>? subscription;
  late final PorchVoice voice;
  Json data = {};
  int? community;
  String? error;
  bool busy = false, sending = false;
  String messageSignature = '';
  int sessionGeneration = 0;
  Json? get user => data['user'] as Json?;
  List<Json> rows(String key) => (data[key] as List? ?? []).cast<Json>();
  List<Json> get communities => rows('communities');
  List<Json> get heads => rows('heads');
  int? get selected => data['selected'] as int?;
  Json? get current => heads.where((h) => h['id'] == selected).firstOrNull;
  Json? get group =>
      communities.where((c) => c['community']['id'] == community).firstOrNull;
  String get title =>
      current == null ? 'Direct messages' : conversationName(current!);

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    voice = PorchVoice(widget.bridge, () {
      if (mounted) setState(() {});
    });
    subscription = widget.bridge.snapshots().listen(
      update,
      onError: (_) {
        if (mounted) {
          setState(
            () => error = 'Connection to the shared engine was interrupted.',
          );
        }
      },
    );
  }

  Future<void> update(Json next) async {
    if (!mounted) return;
    final previousUser = user?['id'];
    final previousSelection = selected;
    if (next.containsKey('error')) {
      setState(() => error = next['error'] as String);
      return;
    }
    setState(() {
      data = next;
      if (previousUser != user?['id']) {
        sessionGeneration++;
        composer.clear();
        community = null;
        messageSignature = '';
      }
      if (previousSelection != selected) composer.clear();
      if (community != null && group == null) community = null;
    });
    if (next['voice'] == null && voice.connected) unawaited(voice.leave());
    final messages = <TextMessage>[];
    for (final m in rows('messages').reversed) {
      final content = m['content'] as Json?;
      messages.add(
        TextMessage(
          id: 'm${m['id']}',
          authorId: '${m['author_id']}',
          text: m['deleted'] == true
              ? 'Message deleted'
              : (content?['text'] as String? ?? ''),
          createdAt: DateTime.tryParse(m['created_at'] as String? ?? ''),
          metadata: {'attachments': m['attachments'] ?? []},
        ),
      );
    }
    for (final p in rows(
      'pending',
    ).where((p) => p['conversation_id'] == selected)) {
      final request = p['request'] as Json;
      messages.add(
        TextMessage(
          id: 'p${request['nonce']}',
          authorId: '${user?['id']}',
          text: request['content']['text'] as String,
          metadata: {'pending': p['state']},
        ),
      );
    }
    final signature =
        '$sessionGeneration/$selected/${messages.map((m) => '${m.id}|${m.text}|${m.metadata}').join('\n')}';
    if (signature != messageSignature) {
      messageSignature = signature;
      await chat.setMessages(messages);
    }
    if (mounted && next['voice_error'] == true) {
      unawaited(voice.leave());
      setState(() => error = 'Voice disconnected. Join the room again.');
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.detached ||
        state == AppLifecycleState.paused) {
      unawaited(voice.leave());
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    unawaited(subscription?.cancel());
    unawaited(voice.leave());
    for (final controller in [origin, username, password, composer]) {
      controller.dispose();
    }
    chat.dispose();
    super.dispose();
  }

  Future<void> action(
    Future<void> Function() f, {
    String failure = 'Action failed. Try again.',
  }) async {
    try {
      await f();
    } catch (_) {
      if (mounted) setState(() => error = failure);
    }
  }

  Future<void> login() async {
    if (busy) return;
    setState(() {
      busy = true;
      error = null;
    });
    final secret = password.text;
    password.clear();
    await action(
      () => widget.bridge.login(
        origin.text.trim(),
        username.text.trim(),
        secret,
        widget.cacheRoot,
      ),
      failure: 'Sign in failed. Check your server, username and password.',
    );
    if (mounted) setState(() => busy = false);
  }

  Future<void> signOut() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Sign out?'),
        content: const Text(
          'Local queued messages will be removed. Some may already have been delivered.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Sign out'),
          ),
        ],
      ),
    );
    if (confirmed != true) return;
    composer.clear();
    await voice.leave();
    await widget.bridge.logout();
  }

  Future<void> select(int id) async {
    if (selected == id) return;
    await action(
      () => widget.bridge.select(id),
      failure: 'Conversation unavailable.',
    );
  }

  void section(int? id) {
    setState(() => community = id);
    final candidates = heads.where((h) => h['community_id'] == id);
    if (candidates.isNotEmpty && !candidates.any((h) => h['id'] == selected)) {
      unawaited(select(candidates.first['id'] as int));
    }
  }

  String conversationName(Json head) {
    if (head['community_id'] != null) return head['name'] as String;
    final peers = (head['participants'] as List? ?? []).where(
      (id) => id != user?['id'],
    );
    return peers.isEmpty
        ? head['name'] as String
        : peers.map((id) => userName('$id')).join(', ');
  }

  Future<void> send() async {
    final text = composer.text;
    final id = selected;
    if (sending || id == null || text.trim().isEmpty) return;
    final generation = sessionGeneration;
    setState(() => sending = true);
    try {
      await widget.bridge.send(id, text);
      if (mounted &&
          generation == sessionGeneration &&
          selected == id &&
          composer.text == text) {
        composer.clear();
      }
    } catch (_) {
      if (mounted) {
        setState(
          () =>
              error = 'Message could not be queued. Your draft has been kept.',
        );
      }
    }
    if (mounted) setState(() => sending = false);
  }

  Widget txt(
    String text, {
    double size = 14,
    Color color = cream,
    FontWeight weight = FontWeight.w500,
  }) => Text(
    text,
    overflow: TextOverflow.ellipsis,
    style: TextStyle(fontSize: size, color: color, fontWeight: weight),
  );

  String userName(String id) {
    if ('${user?['id']}' == id) return user?['username'] as String? ?? id;
    for (final c in communities) {
      final names = c['usernames'] as Json? ?? {};
      if (names[id] is String) return names[id] as String;
    }
    return 'User $id';
  }

  Widget avatar(String name, {double size = 32}) => Container(
    width: size,
    height: size,
    decoration: const BoxDecoration(
      color: Color(0xff66513b),
      shape: BoxShape.circle,
    ),
    alignment: Alignment.center,
    child: txt(
      name.isEmpty ? '?' : name.characters.first.toUpperCase(),
      size: size * .4,
      weight: FontWeight.w700,
    ),
  );

  Widget icon(IconData value, String label, VoidCallback? tap) => IconButton(
    tooltip: label,
    onPressed: tap,
    icon: Icon(value, size: 19, color: muted),
  );

  Widget row(
    String label,
    IconData value,
    VoidCallback tap, {
    bool active = false,
  }) => Padding(
    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 2),
    child: Material(
      color: active ? const Color(0xff35291b) : Colors.transparent,
      borderRadius: BorderRadius.circular(6),
      child: ListTile(
        dense: true,
        onTap: tap,
        leading: Icon(value, size: 18, color: active ? amber : muted),
        title: txt(label, color: active ? cream : muted),
      ),
    ),
  );

  Widget loginView() => Center(
    child: SizedBox(
      width: 380,
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            txt('Porch', size: 36, weight: FontWeight.w800),
            const SizedBox(height: 32),
            TextField(
              controller: origin,
              decoration: const InputDecoration(labelText: 'Server'),
            ),
            const SizedBox(height: 16),
            TextField(
              controller: username,
              autofillHints: const [AutofillHints.username],
              decoration: const InputDecoration(labelText: 'Username'),
            ),
            const SizedBox(height: 16),
            TextField(
              controller: password,
              obscureText: true,
              enableSuggestions: false,
              autocorrect: false,
              decoration: const InputDecoration(labelText: 'Password'),
              onSubmitted: (_) => login(),
            ),
            const SizedBox(height: 28),
            FilledButton(
              onPressed: busy ? null : login,
              child: Text(busy ? 'Signing in' : 'Sign in'),
            ),
            if (error != null)
              Padding(
                padding: const EdgeInsets.only(top: 18),
                child: Text(error!, style: const TextStyle(color: amber)),
              ),
          ],
        ),
      ),
    ),
  );

  Widget topBar() => Container(
    height: 54,
    decoration: const BoxDecoration(
      color: panel,
      border: Border(bottom: BorderSide(color: line)),
    ),
    child: Row(
      children: [
        InkWell(
          onTap: () => section(null),
          child: SizedBox(
            width: 260,
            child: Padding(
              padding: const EdgeInsets.only(left: 26),
              child: txt('Porch', size: 24, weight: FontWeight.w800),
            ),
          ),
        ),
        Expanded(
          child: ListView(
            scrollDirection: Axis.horizontal,
            children: [
              for (final c in communities)
                InkWell(
                  onTap: () => section(c['community']['id'] as int),
                  child: Container(
                    alignment: Alignment.center,
                    padding: const EdgeInsets.symmetric(horizontal: 24),
                    decoration: BoxDecoration(
                      border: Border(
                        bottom: BorderSide(
                          width: 2,
                          color: community == c['community']['id']
                              ? amber
                              : Colors.transparent,
                        ),
                      ),
                    ),
                    child: txt(c['community']['name'] as String),
                  ),
                ),
            ],
          ),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 20),
          child: txt(
            data['connection'] as String? ?? '',
            size: 12,
            color: muted,
          ),
        ),
      ],
    ),
  );

  Widget sidebar() => Container(
    width: 260,
    decoration: const BoxDecoration(
      color: panel,
      border: Border(right: BorderSide(color: line)),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          height: 66,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 24),
            child: Align(
              alignment: Alignment.centerLeft,
              child: txt(
                group?['community']['name'] as String? ?? 'Home',
                size: 17,
                weight: FontWeight.w700,
              ),
            ),
          ),
        ),
        Expanded(
          child: ListView(
            children: [
              if (group == null) ...[
                Padding(
                  padding: const EdgeInsets.fromLTRB(24, 20, 0, 12),
                  child: txt(
                    'DIRECT MESSAGES',
                    size: 10,
                    color: muted,
                    weight: FontWeight.w700,
                  ),
                ),
                for (final h in heads.where((h) => h['community_id'] == null))
                  row(
                    conversationName(h),
                    Icons.chat_bubble_outline,
                    () => select(h['id'] as int),
                    active: selected == h['id'],
                  ),
              ] else ...[
                for (final h in heads.where(
                  (h) => h['community_id'] == community,
                ))
                  row(
                    h['name'] as String,
                    Icons.tag,
                    () => select(h['id'] as int),
                    active: selected == h['id'],
                  ),
                Padding(
                  padding: const EdgeInsets.fromLTRB(24, 28, 0, 12),
                  child: txt(
                    'VOICE ROOMS',
                    size: 10,
                    color: muted,
                    weight: FontWeight.w700,
                  ),
                ),
                for (final ch
                    in (group!['channels'] as List).cast<Json>().where(
                      (c) => c['kind'] == 'voice',
                    ))
                  row(
                    ch['name'] as String,
                    Icons.volume_up_outlined,
                    () => action(
                      () => voice.join(community!, ch['id'] as int),
                      failure:
                          'Could not join voice. Check your microphone and try again.',
                    ),
                    active:
                        data['voice']?['channel']?['channel_id'] == ch['id'] &&
                        data['voice']?['channel']?['community_id'] == community,
                  ),
              ],
            ],
          ),
        ),
        if (data['voice'] != null)
          Container(
            padding: const EdgeInsets.all(12),
            decoration: const BoxDecoration(
              border: Border(top: BorderSide(color: line)),
            ),
            child: Column(
              children: [
                Row(
                  children: [
                    Expanded(
                      child: txt('Voice connected', size: 12, color: amber),
                    ),
                    icon(
                      Icons.call_end_outlined,
                      'Leave voice',
                      () => action(voice.leave),
                    ),
                  ],
                ),
                Row(
                  children: [
                    icon(
                      voice.muted ? Icons.mic_off_outlined : Icons.mic_none,
                      voice.muted ? 'Unmute' : 'Mute',
                      () => action(
                        () => voice.flags(!voice.muted, voice.deafened),
                        failure: 'Voice disconnected. Join again.',
                      ),
                    ),
                    icon(
                      voice.deafened
                          ? Icons.headset_off_outlined
                          : Icons.headphones_outlined,
                      voice.deafened ? 'Undeafen' : 'Deafen',
                      () => action(
                        () => voice.flags(voice.muted, !voice.deafened),
                        failure: 'Voice disconnected. Join again.',
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
        Container(
          height: 72,
          padding: const EdgeInsets.symmetric(horizontal: 18),
          decoration: const BoxDecoration(
            border: Border(top: BorderSide(color: line)),
          ),
          child: Row(
            children: [
              avatar(user?['username'] as String? ?? ''),
              const SizedBox(width: 10),
              Expanded(
                child: txt(user?['username'] as String? ?? '', size: 13),
              ),
              icon(Icons.logout, 'Sign out', signOut),
            ],
          ),
        ),
      ],
    ),
  );

  Widget message(TextMessage m) => Padding(
    padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 14),
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        avatar(userName(m.authorId), size: 36),
        const SizedBox(width: 14),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  txt(userName(m.authorId), weight: FontWeight.w700),
                  const SizedBox(width: 12),
                  txt(
                    m.createdAt == null
                        ? (m.metadata?['pending'] as String? ?? '')
                        : TimeOfDay.fromDateTime(
                            m.createdAt!.toLocal(),
                          ).format(context),
                    size: 10,
                    color: muted,
                  ),
                ],
              ),
              const SizedBox(height: 7),
              SelectableText(
                m.text,
                style: const TextStyle(fontSize: 14, height: 1.6, color: cream),
              ),
              for (final a
                  in (m.metadata?['attachments'] as List? ?? []).cast<Json>())
                Container(
                  margin: const EdgeInsets.only(top: 10),
                  padding: const EdgeInsets.all(14),
                  constraints: const BoxConstraints(maxWidth: 360),
                  decoration: BoxDecoration(
                    color: panel,
                    border: Border.all(color: line),
                    borderRadius: BorderRadius.circular(7),
                  ),
                  child: Row(
                    children: [
                      const Icon(
                        Icons.insert_drive_file_outlined,
                        color: amber,
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: txt(
                          a['filename'] as String? ?? 'Attachment',
                          size: 12,
                        ),
                      ),
                    ],
                  ),
                ),
            ],
          ),
        ),
      ],
    ),
  );

  Widget timeline() => Chat(
    currentUserId: '${user?['id']}',
    resolveUser: (id) async => User(id: id, name: userName(id)),
    chatController: chat,
    backgroundColor: bg,
    theme: ChatTheme.fromThemeData(Theme.of(context)),
    builders: Builders(
      chatMessageBuilder:
          (
            context,
            message,
            index,
            animation,
            child, {
            isRemoved,
            required isSentByMe,
            groupStatus,
          }) => child,
      textMessageBuilder:
          (context, m, index, {required isSentByMe, groupStatus}) => message(m),
      chatAnimatedListBuilder: (context, itemBuilder) => ChatAnimatedList(
        itemBuilder: itemBuilder,
        bottomPadding: 110,
        topPadding: 20,
      ),
      composerBuilder: (context) => Positioned(
        left: 0,
        right: 0,
        bottom: 0,
        child: Container(
          margin: const EdgeInsets.fromLTRB(32, 16, 32, 26),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
          decoration: BoxDecoration(
            color: panel,
            border: Border.all(color: line),
            borderRadius: BorderRadius.circular(8),
          ),
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  key: const Key('composer'),
                  controller: composer,
                  onSubmitted: (_) => send(),
                  maxLength: 8000,
                  style: const TextStyle(fontSize: 14),
                  decoration: InputDecoration(
                    border: InputBorder.none,
                    counterText: '',
                    hintText: 'Message $title',
                    hintStyle: const TextStyle(color: muted, fontSize: 14),
                  ),
                ),
              ),
              icon(Icons.send_outlined, 'Send message', sending ? null : send),
            ],
          ),
        ),
      ),
    ),
  );

  @override
  Widget build(BuildContext context) => Scaffold(
    body: user == null
        ? loginView()
        : Column(
            children: [
              topBar(),
              if (error != null || data['notice'] != null)
                MaterialBanner(
                  backgroundColor: panel,
                  content: Text(error ?? data['notice'] as String),
                  actions: [
                    TextButton(
                      onPressed: () => setState(() {
                        error = null;
                        data.remove('notice');
                      }),
                      child: const Text('Dismiss'),
                    ),
                  ],
                ),
              Expanded(
                child: Row(
                  children: [
                    sidebar(),
                    Expanded(
                      child: Column(
                        children: [
                          Container(
                            height: 66,
                            padding: const EdgeInsets.symmetric(horizontal: 32),
                            decoration: const BoxDecoration(
                              border: Border(bottom: BorderSide(color: line)),
                            ),
                            child: Row(
                              children: [
                                Icon(
                                  current?['community_id'] == null
                                      ? Icons.alternate_email
                                      : Icons.tag,
                                  color: muted,
                                  size: 21,
                                ),
                                const SizedBox(width: 12),
                                txt(title, size: 16, weight: FontWeight.w700),
                              ],
                            ),
                          ),
                          Expanded(
                            child: selected == null
                                ? const Center(child: Text('No conversations'))
                                : timeline(),
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
  );
}
