import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:google_sign_in/google_sign_in.dart';
import 'package:web_socket_client/web_socket_client.dart';

class Server {
  Server() {
    _googleSignInSubscription = _googleSignIn.onCurrentUserChanged.listen(
      (GoogleSignInAccount? currentUser) {
        assert(currentUser == _googleSignIn.currentUser);
        _account.value = currentUser;
        sendLogin();
      },
    );
    _socket.messages.listen((Object? message) {
      print('server says: $message');
    });
    _socket.connection.listen((ConnectionState state) {
      _connected.value = (state is Connected || state is Reconnected);
      sendLogin();
    });
    _googleSignIn.signInSilently();
  }

  final WebSocket _socket = WebSocket(Uri.parse('wss://polyplot.fun:1979'));
  final GoogleSignIn _googleSignIn = GoogleSignIn();

  late final StreamSubscription _googleSignInSubscription;

  ValueListenable<GoogleSignInAccount?> get account => _account;
  final ValueNotifier<GoogleSignInAccount?> _account = ValueNotifier<GoogleSignInAccount?>(null);

  ValueListenable<bool> get connected => _connected;
  final ValueNotifier<bool> _connected = ValueNotifier<bool>(false);

  void triggerSignIn() {
    _googleSignIn.signIn();
  }

  void triggerSignOut() {
    _googleSignIn.signOut();
  }

  void sendLogin() {
    if (_connected.value && _account.value != null) {
      _account.value!.authentication.then((GoogleSignInAuthentication auth) {
        _socket.send('login\ngoogle\n${auth.idToken}');
      });
    }
  }

  void dispose() {
    _googleSignInSubscription.cancel();
    _socket.close();
  }
}
