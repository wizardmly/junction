# IntelliJ Platform icons

The SVG files in `src/` are the IntelliJ Platform's new UI ("expui") icons,
copied unchanged from JetBrains' intellij-community repository
(https://github.com/JetBrains/intellij-community), except for
`module-android*.svg` and `gradle-kotlin*.svg`, which combine two of them.

Copyright 2000-2024 JetBrains s.r.o. and contributors.
Licensed under the Apache License, Version 2.0:
https://www.apache.org/licenses/LICENSE-2.0

`tools/as_icons.py` splits each icon into one-color layers under `gen/`
(generated, with `src/ui/as_icons_gen.rs`); rerun it after adding an icon.
