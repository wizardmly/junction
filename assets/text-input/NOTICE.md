# Text input helper

`junction-text-input.jar` is Junction's own code (source in
`tools/text-input/src`, built with `tools/text-input/build.sh`). Running
Devices pushes it to the device and runs it as the shell user with
`app_process` to set typed non-ASCII text (Chinese, emoji, …) into the
focused text field through a UiAutomation, the way `uiautomator` connects,
so neither the device's keyboard nor its clipboard is involved.
