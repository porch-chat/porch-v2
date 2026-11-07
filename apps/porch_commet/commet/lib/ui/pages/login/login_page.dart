import 'dart:async';

import 'package:commet/client/auth.dart';
import 'package:commet/client/client.dart';
import 'package:commet/client/porch/porch_client.dart';
import 'package:commet/config/global_config.dart';
import 'package:path_provider/path_provider.dart';
import 'package:porch_flutter/bridge.dart';
import 'package:commet/main.dart';
import 'package:commet/ui/pages/login/login_page_view.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:commet/ui/navigation/adaptive_dialog.dart';
import 'package:tiamat/tiamat.dart' as tiamat;

class LoginPage extends StatefulWidget {
  const LoginPage({super.key, this.onSuccess, this.canNavigateBack = false});
  final bool canNavigateBack;
  final Function(Client loggedInClient)? onSuccess;

  @override
  State<LoginPage> createState() => LoginPageState();
}

class LoginPageState extends State<LoginPage> {
  String get messageLoginFailed => Intl.message(
        "Login Failed...",
        name: "messageLoginFailed",
        desc: "Generic text to show that an attempted login has failed",
      );

  String get messageAlreadyLoggedIn => Intl.message(
        "You have already logged in to this account",
        name: "messageAlreadyLoggedIn",
        desc:
            "An error message displayed when the user attempts to add an account which has already been logged in to on this device",
      );

  String get messageInvalidUsernameOrPassword => Intl.message(
      "Invalid username or password",
      name: "messageInvalidUsernameOrPassword",
      desc:
          "An error message displayed when the user attempts to log into an account using the wrong username/password combination");

  StreamSubscription? progressSubscription;
  double? progress;
  List<LoginFlow>? loginFlows;
  Client? loginClient;

  bool loadingServerInfo = false;
  bool isServerValid = false;
  bool isLoggingIn = false;

  @override
  void initState() {
    getApplicationSupportDirectory().then((dir) {
      final client = PorchClient(RustPorchBridge(), '${dir.path}/porch');
      loginClient = client;

      progressSubscription = loginClient!.connectionStatusChanged.stream.listen(
        onLoginProgressChanged,
      );
      useConfiguredServer();
    });

    super.initState();
  }

  @override
  Widget build(BuildContext context) {
    return LoginPageView(
      canNavigateBack: widget.canNavigateBack,
      progress: progress,
      flows: loginFlows,
      doSsoLogin: doSsoLogin,
      doPasswordLogin: doPasswordLogin,
      isLoggingIn: isLoggingIn,
      loadingServerInfo: loadingServerInfo,
      hasSsoSupport: loginFlows?.whereType<SsoLoginFlow>().isNotEmpty == true,
      hasPasswordSupport:
          loginFlows?.whereType<PasswordLoginFlow>().isNotEmpty == true,
      isServerValid: isServerValid,
    );
  }

  Future<void> doLogin(Future<LoginResult> Function() login) async {
    if (loginClient == null) return;
    if (isServerValid == false) {
      return;
    }

    setState(() {
      isLoggingIn = true;
    });

    LoginResult? result;

    try {
      result = await login();
    } catch (_) {}

    if (!(result is LoginResultSuccess)) {
      setState(() {
        isLoggingIn = false;
      });
    }

    String? message = switch (result) {
      LoginResultSuccess _ => null,
      LoginResultError e => e.errorMessage,
      LoginResultCancelled _ => "Login Cancelled",
      LoginResultAlreadyLoggedIn _ => messageAlreadyLoggedIn,
      LoginResultFailed _ => messageLoginFailed,
      LoginResult() => throw UnimplementedError(),
      null => throw UnimplementedError(),
    };

    if (message != null) {
      if (mounted) {
        AdaptiveDialog.show(
          context,
          title: "Login failed",
          builder: (_) => tiamat.Text(
            message,
          ),
        );
      }
    }

    if (result is LoginResultSuccess) {
      clientManager?.addClient(loginClient!);
      widget.onSuccess?.call(loginClient!);
    }
  }

  Future<void> doSsoLogin(SsoLoginFlow flow) async {
    if (loginClient == null) return;
    await doLogin(() => loginClient!.executeLoginFlow(flow));
  }

  Future<void> doPasswordLogin(
    PasswordLoginFlow flow,
    String username,
    String password,
  ) async {
    if (loginClient == null) return;
    flow.username = username;
    flow.password = password;

    await doLogin(() => loginClient!.executeLoginFlow(flow));
  }

  void onLoginProgressChanged(ClientConnectionStatusUpdate event) {
    if (!mounted) return;
    setState(() {
      progress = event.progress;
    });
  }

  Future<void> useConfiguredServer() async {
    final uri = PorchClient.parseServer(GlobalConfig.porchServer);
    if (loginClient == null || uri == null) return;
    final result = await loginClient!.setHomeserver(uri);
    if (!mounted) return;
    setState(() {
      loadingServerInfo = false;
      isServerValid = result.$1;
      loginFlows = result.$2;
    });
  }
}
