#!/usr/bin/env python3
"""Sign a built Quiet.md device app with an explicit iCloud development profile."""
import argparse
import datetime
import fnmatch
import hashlib
import pathlib
import plistlib
import re
import shutil
import subprocess
import tempfile

here = pathlib.Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("profile", type=pathlib.Path)
parser.add_argument("--app", type=pathlib.Path, default=here.parents[1] / "target/ios/device/Quiet.md Prototype.app")
args = parser.parse_args()
app = args.app.resolve()
profile = args.profile.resolve()
info = plistlib.loads((app / "Info.plist").read_bytes())
bundle = plistlib.loads((here / "Info.plist").read_bytes())["CFBundleIdentifier"]
if info["CFBundleIdentifier"] != bundle: parser.error("App bundle ID does not match prototypes/ios/Info.plist")
p = plistlib.loads(subprocess.check_output(["security", "cms", "-D", "-i", str(profile)]))
allowed = p["Entitlements"]
team = p["TeamIdentifier"][0]
prefix = p["ApplicationIdentifierPrefix"][0] + "."
if p["ExpirationDate"] <= datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None): parser.error("Provisioning profile has expired")
if allowed.get("application-identifier") != prefix + bundle: parser.error("An explicit profile for this app is required; wildcard profiles cannot enable iCloud")
if not allowed.get("get-task-allow") or not p.get("ProvisionedDevices"): parser.error("An iOS development profile with registered devices is required")
entitlements = plistlib.loads((here / "QuietMd.entitlements").read_bytes())
entitlements["com.apple.developer.ubiquity-kvstore-identifier"] = team + "." + bundle
entitlements.update({"application-identifier": prefix + bundle, "com.apple.developer.team-identifier": team,
    "get-task-allow": True, "keychain-access-groups": [prefix + bundle],
    "com.apple.developer.icloud-container-environment": "Development"})
for key, value in entitlements.items():
    options = allowed.get(key, [])
    options = options if isinstance(options, list) else [options]
    for item in value if isinstance(value, list) else [value]:
        if not any(fnmatch.fnmatchcase(item, option) if isinstance(item, str) and isinstance(option, str) else item == option for option in options):
            parser.error(f"Profile does not permit {key}: {item}")
identities = subprocess.check_output(["security", "find-identity", "-v", "-p", "codesigning"], text=True)
available = set(re.findall(r"\b[0-9A-F]{40}\b", identities))
identity = next((hashlib.sha1(cert).hexdigest().upper() for cert in p["DeveloperCertificates"] if hashlib.sha1(cert).hexdigest().upper() in available), None)
if not identity: parser.error("No local signing identity matches this profile")
with tempfile.TemporaryDirectory(prefix="quiet-md-sign-") as temp:
    ent = pathlib.Path(temp) / "entitlements.plist"
    ent.write_bytes(plistlib.dumps(entitlements))
    if profile != app / "embedded.mobileprovision": shutil.copyfile(profile, app / "embedded.mobileprovision")
    subprocess.run(["codesign", "--force", "--sign", identity, "--entitlements", str(ent), "--generate-entitlement-der", str(app)], check=True)
    subprocess.run(["codesign", "--verify", "--strict", str(app)], check=True)
    signed = plistlib.loads(subprocess.check_output(["codesign", "-d", "--entitlements", "-", "--xml", str(app)], stderr=subprocess.DEVNULL))
    if signed != entitlements: raise SystemExit("Signed entitlements differ from the requested configuration")
print(f"Signed {app}\nProfile: {p['Name']} ({p['UUID']})\nExpires: {p['ExpirationDate']} UTC")
