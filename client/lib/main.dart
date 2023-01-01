import 'package:flutter/material.dart';
import 'package:flutter_signin_button/flutter_signin_button.dart';
import 'package:google_sign_in/google_sign_in.dart';

import 'server.dart';

void main() {
  runApp(MyApp(server: Server()));
}

class MyApp extends StatelessWidget {
  const MyApp({super.key, required this.server});

  final Server server;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Polyplot',
      home: HomePage(server: server),
    );
  }
}

class HomePage extends StatefulWidget {
  const HomePage({super.key, required this.server});

  final Server server;

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  @override
  Widget build(BuildContext context) {
    return Center(
      child: ValueListenableBuilder(
        valueListenable: widget.server.account,
        builder: (BuildContext context, GoogleSignInAccount? account, Widget? child) {
          return account == null
               ? SizedBox(height: 32.0, child: SignInButton(Buttons.Google, onPressed: widget.server.triggerSignIn))
               : GoogleUserCircleAvatar(identity: account);
        },
      ),
    );
  }
}
