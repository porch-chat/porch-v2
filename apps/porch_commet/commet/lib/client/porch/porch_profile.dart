import 'dart:typed_data';

import 'package:commet/client/components/profile/profile_component.dart';
import 'package:commet/client/porch/porch_client.dart';
import 'package:flutter/material.dart';

/// Read-only profile for a Porch user. Editing is not supported yet.
class PorchProfile implements Profile {
  final PorchClient client;
  @override
  final String identifier;
  PorchProfile(this.client, this.identifier);

  @override
  String get userName => client.peerOf(identifier).userName;
  @override
  String get displayName => userName;
  @override
  String? get detail => null;
  @override
  ImageProvider? get avatar => null;
  @override
  ImageProvider? get banner => null;
  @override
  Color get defaultColor => client.peerOf(identifier).defaultColor;
  @override
  String get source => '';
}

class PorchProfileComponent implements UserProfileComponent<PorchClient> {
  @override
  final PorchClient client;
  PorchProfileComponent(this.client);

  @override
  Future<Profile> getProfile(String identifier) async =>
      PorchProfile(client, identifier);

  Never _no() => throw UnsupportedError('Profile editing is not supported yet');
  @override
  Future<void> setBanner(Uint8List bytes) async => _no();
  @override
  Future<void> setProfileColorScheme(Color color, Brightness brightness) async =>
      _no();
  @override
  Future<void> setStatus(String? status) async => _no();
  @override
  Future<void> setTimezone(String timezone) async => _no();
  @override
  Future<void> setBio(String bio) async => _no();
  @override
  Future<void> removeBio() async => _no();
  @override
  Future<void> removeBanner() async => _no();
  @override
  Future<void> removeTimezone() async => _no();
  @override
  Future<List<ProfileBadge>> getAvailableBadges() async => const [];
  @override
  Future<void> setProfileBadges(List<ProfileBadge> badges) async => _no();
}
