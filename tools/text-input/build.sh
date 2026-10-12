#!/bin/sh
# Builds assets/text-input/junction-text-input.jar (a dex jar for app_process).
# Needs a JDK, an Android framework jar to compile against and dx, e.g. from
# Maven Central: org.robolectric:android-all:14-robolectric-10818077 and
# com.jakewharton.android.repackaged:dalvik-dx:14.0.0_r21.
#   tools/text-input/build.sh <android-all.jar> <dalvik-dx.jar>
set -e
here=$(cd "$(dirname "$0")" && pwd)
out="$here/../../assets/text-input/junction-text-input.jar"
work=$(mktemp -d)
javac --release 8 -nowarn -cp "$1" -d "$work/classes" "$here/src/dev/junction/TextInput.java"
java -cp "$2" com.android.dx.command.Main --dex --min-sdk-version=21 --output="$out" "$work/classes"
rm -rf "$work"
echo "$out"
