import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_signin_button/flutter_signin_button.dart';
import 'package:google_sign_in/google_sign_in.dart';
import 'package:web_socket_client/web_socket_client.dart';

final GoogleSignIn googleSignIn = GoogleSignIn();
late final WebSocket server;

void main() {
  server = WebSocket(Uri.parse('wss://polyplot.fun:1979'));
  server.messages.listen((Object? message) {
    print('server says: $message');
  });
  runApp(const MyApp());
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return const MaterialApp(
      title: 'Polyplot',
      home: HomePage(title: 'Polyplot'),
    );
  }
}

class HomePage extends StatefulWidget {
  const HomePage({super.key, required this.title});

  final String title;

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {

  late final StreamSubscription _googleSignInSubscription;

  @override
  void initState() {
    super.initState();
    _googleSignInSubscription = googleSignIn.onCurrentUserChanged.listen(
      (GoogleSignInAccount? account) {
        setState(() {
          // googleSignIn.currentUser changed
          if (account != null) {
            googleSignIn.currentUser!.authentication.then((GoogleSignInAuthentication auth) {
              server.send('login\ngoogle\n${auth.idToken}');
            });
          }
        });
      },
    );
  }

  @override
  void dispose() {
    _googleSignInSubscription.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text(widget.title),
      ),
      body: Center(
        child: googleSignIn.currentUser == null ? SignInButton(
          Buttons.Google,
          onPressed: () {
            googleSignIn.signIn();
          },
        ) : GoogleUserCircleAvatar(identity: googleSignIn.currentUser!),
      ),
    );
  }
}
