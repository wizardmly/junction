//! Android SDK packages as Android Studio's SDK Manager sees them: the
//! installed ones (each folder's `package.xml`), the ones Google's
//! repository offers (`repository2-3.xml`), and the rows of the SDK
//! Platforms and SDK Tools tabs with what Apply would change.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A package revision: `35.0.0`, `3`, `36.0.0 rc4`. Its precision is how
/// many parts the XML gave, as `Revision.toShortString` prints it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Revision {
    pub major: u32,
    pub minor: u32,
    pub micro: u32,
    pub preview: Option<u32>,
    precision: u8,
}

impl Revision {
    #[cfg(test)]
    pub fn new(major: u32, minor: u32, micro: u32, preview: Option<u32>) -> Self {
        Self { major, minor, micro, preview, precision: 3 }
    }

    /// `35.0.1`, `3`, `1.0.0-rc2` as `sdkmanager` takes them.
    #[cfg(test)]
    pub fn parse(text: &str) -> Option<Self> {
        let (numbers, preview) = match text.split_once(|c: char| c == ' ' || c == '-') {
            Some((n, p)) => (n, p.trim_start_matches(|c: char| !c.is_ascii_digit()).parse().ok()),
            None => (text, None),
        };
        let parts: Vec<u32> = numbers.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
        let at = |i: usize| parts.get(i).copied().unwrap_or(0);
        Some(Self { major: at(0), minor: at(1), micro: at(2), preview, precision: parts.len().clamp(1, 3) as u8 })
    }

    fn key(&self) -> (u32, u32, u32, u32) {
        // A final release comes after its previews.
        (self.major, self.minor, self.micro, self.preview.unwrap_or(u32::MAX))
    }
}

impl PartialOrd for Revision {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Revision {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.major)?;
        if self.precision >= 2 {
            write!(f, ".{}", self.minor)?;
        }
        if self.precision >= 3 {
            write!(f, ".{}", self.micro)?;
        }
        if let Some(p) = self.preview {
            write!(f, " rc{p}")?;
        }
        Ok(())
    }
}

/// What the package's `type-details` say it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Platform,
    Sources,
    SystemImage,
    AddOn,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Archive {
    /// Relative to the repository's folder, or a whole URL.
    pub url: String,
    pub size: u64,
    pub sha1: String,
    pub host_os: Option<String>,
    pub host_arch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// `platforms;android-34`, `build-tools;35.0.0`.
    pub path: String,
    pub display_name: String,
    pub revision: Revision,
    pub kind: Kind,
    /// The API level of a platform, its sources or a system image.
    pub api: Option<String>,
    pub obsolete: bool,
    pub license: Option<String>,
    pub archives: Vec<Archive>,
    /// The package's elements but its archives, for its `package.xml`.
    pub local_xml: String,
}

/// What Google's repository offers: the newest stable revision of each
/// package, and the license texts by id.
#[derive(Clone, Debug, Default)]
pub struct Repository {
    pub packages: Vec<Package>,
    pub licenses: BTreeMap<String, String>,
    /// The root's namespace declarations, which the elements kept in
    /// `local_xml` use.
    pub namespaces: String,
}

fn text_of(node: roxmltree::Node, tag: &str) -> Option<String> {
    node.children().find(|n| n.has_tag_name(tag)).and_then(|n| n.text()).map(|t| t.trim().to_owned())
}

fn revision_of(node: roxmltree::Node) -> Revision {
    let Some(rev) = node.children().find(|n| n.has_tag_name("revision")) else { return Revision::default() };
    let part = |tag: &str| text_of(rev, tag).and_then(|t| t.parse::<u32>().ok());
    let given = [part("major"), part("minor"), part("micro")];
    let precision = given.iter().rposition(Option::is_some).map_or(1, |i| i + 1) as u8;
    Revision { major: given[0].unwrap_or(0), minor: given[1].unwrap_or(0), micro: given[2].unwrap_or(0), preview: part("preview"), precision }
}

fn kind_and_api(node: roxmltree::Node) -> (Kind, Option<String>) {
    let Some(details) = node.children().find(|n| n.has_tag_name("type-details")) else { return (Kind::Other, None) };
    let ty = details.attributes().find(|a| a.name() == "type").map(|a| a.value()).unwrap_or_default();
    let ty = ty.rsplit(':').next().unwrap_or_default();
    let kind = match ty {
        "platformDetailsType" => Kind::Platform,
        "sourceDetailsType" => Kind::Sources,
        "sysImgDetailsType" => Kind::SystemImage,
        "addonDetailsType" => Kind::AddOn,
        _ => Kind::Other,
    };
    // A preview platform's codename stands for its level.
    let api = text_of(details, "codename").filter(|c| !c.is_empty()).or_else(|| text_of(details, "api-level"));
    let api = api.map(|level| match text_of(details, "extension-level").filter(|_| kind == Kind::Platform && !text_of(details, "base-extension").is_some_and(|b| b == "true")) {
        Some(ext) => format!("{level} ext. {ext}"),
        None => level,
    });
    (kind, api)
}

fn package_of(xml: &str, node: roxmltree::Node) -> Package {
    let (kind, api) = kind_and_api(node);
    let license = node.children().find(|n| n.has_tag_name("uses-license")).and_then(|n| n.attribute("ref")).map(str::to_owned);
    let archives = node
        .descendants()
        .filter(|n| n.has_tag_name("archive"))
        .filter_map(|a| {
            let complete = a.children().find(|n| n.has_tag_name("complete"))?;
            let sha1 = complete.children().find(|n| n.has_tag_name("checksum") && n.attribute("type").is_none_or(|t| t == "sha1")).and_then(|n| n.text())?;
            Some(Archive {
                url: text_of(complete, "url")?,
                size: text_of(complete, "size").and_then(|s| s.parse().ok()).unwrap_or(0),
                sha1: sha1.trim().to_lowercase(),
                host_os: text_of(a, "host-os"),
                host_arch: text_of(a, "host-arch"),
            })
        })
        .collect();
    let local_xml = node.children().filter(|n| n.is_element() && !n.has_tag_name("archives") && !n.has_tag_name("channelRef")).map(|n| &xml[n.range()]).collect::<Vec<_>>().join("");
    Package {
        path: node.attribute("path").unwrap_or_default().to_owned(),
        display_name: text_of(node, "display-name").unwrap_or_default(),
        revision: revision_of(node),
        kind,
        api,
        obsolete: node.attribute("obsolete") == Some("true"),
        license,
        archives,
        local_xml,
    }
}

/// An installed package's `package.xml`.
pub fn parse_local(xml: &str) -> Option<Package> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let node = doc.descendants().find(|n| n.has_tag_name("localPackage"))?;
    let package = package_of(xml, node);
    (!package.path.is_empty()).then_some(package)
}

/// The repository's packages on the stable channel, the newest revision of each.
pub fn parse_repository(xml: &str) -> Repository {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Repository::default() };
    let root = doc.root_element();
    let stable: BTreeSet<&str> = root.children().filter(|n| n.has_tag_name("channel") && n.text().map(str::trim) == Some("stable")).filter_map(|n| n.attribute("id")).collect();
    let licenses = root.children().filter(|n| n.has_tag_name("license")).filter_map(|n| Some((n.attribute("id")?.to_owned(), n.text().unwrap_or_default().to_owned()))).collect();
    let mut newest: BTreeMap<String, Package> = BTreeMap::new();
    for node in root.children().filter(|n| n.has_tag_name("remotePackage")) {
        let channel = node.children().find(|n| n.has_tag_name("channelRef")).and_then(|n| n.attribute("ref"));
        if channel.is_some_and(|c| !stable.contains(c)) {
            continue;
        }
        let package = package_of(xml, node);
        if newest.get(&package.path).is_none_or(|p| p.revision < package.revision) {
            newest.insert(package.path.clone(), package);
        }
    }
    let namespaces = root.namespaces().map(|ns| match ns.name() {
        Some(prefix) => format!(" xmlns:{prefix}=\"{}\"", ns.uri()),
        None => format!(" xmlns=\"{}\"", ns.uri()),
    });
    Repository { packages: newest.into_values().collect(), licenses, namespaces: namespaces.collect() }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The `package.xml` an installed package gets, as the SDK Manager writes it.
pub fn package_xml(package: &Package, repository: &Repository) -> String {
    let license = package.license.as_ref().and_then(|id| Some((id, repository.licenses.get(id)?)));
    let license = license.map(|(id, text)| format!("<license id=\"{}\" type=\"text\">{}</license>", escape(id), escape(text))).unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<ns2:repository xmlns:ns2=\"http://schemas.android.com/repository/android/common/02\"{}>{license}<localPackage path=\"{}\" obsolete=\"{}\">{}</localPackage></ns2:repository>\n",
        repository.namespaces,
        escape(&package.path),
        package.obsolete,
        package.local_xml
    )
}

/// The package's folder under the SDK: `platforms;android-34` → `platforms/android-34`.
pub fn relative_dir(path: &str) -> String {
    path.replace(';', "/")
}

/// The archive for this machine (`os` and `arch` as Rust names them); an
/// archive for any machine, else one for this OS on another architecture.
pub fn archive_for<'a>(package: &'a Package, os: &str, arch: &str) -> Option<&'a Archive> {
    let os = match os {
        "macos" => "macosx",
        other => other,
    };
    let arch = match arch {
        "x86_64" => "x64",
        "x86" => "x86",
        other => other,
    };
    let fits_os = |a: &&Archive| a.host_os.as_deref().is_none_or(|o| o == os);
    let fits_arch = |a: &&Archive| a.host_arch.as_deref().is_none_or(|h| h == arch);
    package.archives.iter().filter(fits_os).find(fits_arch).or_else(|| package.archives.iter().find(fits_os))
}

/// What Android Studio calls a release: `Android 14.0 ("UpsideDownCake")`.
pub fn platform_name(api: &str) -> String {
    let level = api.split(|c: char| !c.is_ascii_digit()).next().and_then(|l| l.parse::<u32>().ok());
    let name = match level {
        Some(36) => "16.0 (\"Baklava\")",
        Some(35) => "15.0 (\"VanillaIceCream\")",
        Some(34) => "14.0 (\"UpsideDownCake\")",
        Some(33) => "13.0 (\"Tiramisu\")",
        Some(32) => "12L (\"Sv2\")",
        Some(31) => "12.0 (\"S\")",
        Some(30) => "11.0 (\"R\")",
        Some(29) => "10.0 (\"Q\")",
        Some(28) => "9.0 (\"Pie\")",
        Some(27) => "8.1 (\"Oreo\")",
        Some(26) => "8.0 (\"Oreo\")",
        Some(25) => "7.1.1 (\"Nougat\")",
        Some(24) => "7.0 (\"Nougat\")",
        Some(23) => "6.0 (\"Marshmallow\")",
        Some(22) => "5.1 (\"Lollipop\")",
        Some(21) => "5.0 (\"Lollipop\")",
        Some(20) => "4.4W (\"KitKat Wear\")",
        Some(19) => "4.4 (\"KitKat\")",
        _ => return if level.is_some() { format!("API {api}") } else { format!("Android {api} Preview") },
    };
    match api.split_once(" ext. ") {
        Some((_, ext)) => format!("Android {name}, extension level {ext}"),
        None => format!("Android {name}"),
    }
}

/// A release's order: newest API first, previews (codenames) on top.
fn api_order(api: &str) -> (u32, u32, String) {
    let mut numbers = api.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse::<u32>().unwrap_or(0));
    match api.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        true => (numbers.next().unwrap_or(0), numbers.next().unwrap_or(0), String::new()),
        false => (u32::MAX, 0, api.to_owned()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Installed,
    NotInstalled,
    PartiallyInstalled,
    UpdateAvailable(Revision),
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Installed => f.write_str("Installed"),
            Status::NotInstalled => f.write_str("Not installed"),
            Status::PartiallyInstalled => f.write_str("Partially installed"),
            Status::UpdateAvailable(r) => write!(f, "Update Available: {r}"),
        }
    }
}

/// A row of a tab: a package, a group's heading (no packages of its own),
/// or a summary row standing for its packages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub api: String,
    pub revision: String,
    pub status: Option<Status>,
    /// The packages it stands for; the first is what checking it installs.
    pub packages: Vec<String>,
    pub depth: u8,
}

/// Every package the tabs know: the repository's, and the installed ones
/// it no longer offers.
pub struct Catalog {
    pub remote: BTreeMap<String, Package>,
    pub local: BTreeMap<String, Package>,
}

impl Catalog {
    pub fn new(remote: &[Package], local: &[Package]) -> Self {
        Self { remote: remote.iter().map(|p| (p.path.clone(), p.clone())).collect(), local: local.iter().map(|p| (p.path.clone(), p.clone())).collect() }
    }

    fn get(&self, path: &str) -> Option<&Package> {
        self.local.get(path).or_else(|| self.remote.get(path))
    }

    fn all(&self, hide_obsolete: bool) -> Vec<&Package> {
        let mut paths: BTreeSet<&String> = self.remote.keys().collect();
        paths.extend(self.local.keys());
        paths
            .into_iter()
            .filter_map(|p| self.get(p))
            // An installed obsolete package stays, as Android Studio keeps it.
            .filter(|p| !(hide_obsolete && p.obsolete && !self.local.contains_key(&p.path)))
            .collect()
    }

    pub fn status(&self, path: &str) -> Status {
        match (self.local.get(path), self.remote.get(path)) {
            (Some(local), Some(remote)) if remote.revision > local.revision => Status::UpdateAvailable(remote.revision),
            (Some(_), _) => Status::Installed,
            _ => Status::NotInstalled,
        }
    }

    fn group_status(&self, packages: &[String]) -> Status {
        let statuses: Vec<Status> = packages.iter().map(|p| self.status(p)).collect();
        if let Some(update) = statuses.iter().find(|s| matches!(s, Status::UpdateAvailable(_))) {
            return update.clone();
        }
        match statuses.iter().filter(|s| **s == Status::Installed).count() {
            0 => Status::NotInstalled,
            n if n == statuses.len() => Status::Installed,
            _ => Status::PartiallyInstalled,
        }
    }

    fn package_row(&self, package: &Package, depth: u8) -> Row {
        Row {
            name: package.display_name.clone(),
            api: package.api.clone().unwrap_or_default(),
            revision: package.revision.to_string(),
            status: Some(self.status(&package.path)),
            packages: vec![package.path.clone()],
            depth,
        }
    }

    /// SDK Platforms: a row per release (its platform package), or with
    /// package details, each release's platform, sources and system images.
    pub fn platform_rows(&self, details: bool, hide_obsolete: bool) -> Vec<Row> {
        let mut releases: BTreeMap<(std::cmp::Reverse<(u32, u32, String)>, String), Vec<&Package>> = BTreeMap::new();
        for package in self.all(hide_obsolete) {
            if !matches!(package.kind, Kind::Platform | Kind::Sources | Kind::SystemImage | Kind::AddOn) {
                continue;
            }
            let api = package.api.clone().unwrap_or_default();
            releases.entry((std::cmp::Reverse(api_order(&api)), api)).or_default().push(package);
        }
        let mut rows = Vec::new();
        for ((_, api), mut packages) in releases {
            // The platform first, then its sources, then the rest by name.
            packages.sort_by_key(|p| (p.kind != Kind::Platform, p.kind != Kind::Sources, p.display_name.clone()));
            let name = platform_name(&api);
            if details {
                rows.push(Row { name, api: String::new(), revision: String::new(), status: None, packages: Vec::new(), depth: 0 });
                rows.extend(packages.iter().map(|p| self.package_row(p, 1)));
            } else {
                let Some(platform) = packages.iter().find(|p| p.kind == Kind::Platform) else { continue };
                let paths: Vec<String> = packages.iter().map(|p| p.path.clone()).collect();
                let status = self.group_status(&paths);
                rows.push(Row { name, api: api.clone(), revision: platform.revision.to_string(), status: Some(status), packages: paths, depth: 0 });
            }
        }
        rows
    }

    /// SDK Tools: a row per tool, the newest of a versioned one (Build-Tools,
    /// NDK, CMake); with package details, each version under its tool.
    pub fn tool_rows(&self, details: bool, hide_obsolete: bool) -> Vec<Row> {
        let mut groups: BTreeMap<String, Vec<&Package>> = BTreeMap::new();
        for package in self.all(hide_obsolete).into_iter().filter(|p| p.kind == Kind::Other) {
            let prefix = package.path.split(';').next().unwrap_or_default().to_owned();
            groups.entry(prefix).or_default().push(package);
        }
        let mut named: Vec<(String, Vec<&Package>)> = groups
            .into_iter()
            .map(|(prefix, mut packages)| {
                packages.sort_by(|a, b| b.revision.cmp(&a.revision).then_with(|| a.path.cmp(&b.path)));
                (tool_name(&prefix, &packages), packages)
            })
            .collect();
        named.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        let mut rows = Vec::new();
        for (name, packages) in named {
            if packages.len() == 1 {
                rows.push(self.package_row(packages[0], 0));
            } else if details {
                rows.push(Row { name, api: String::new(), revision: String::new(), status: None, packages: Vec::new(), depth: 0 });
                rows.extend(packages.iter().map(|p| self.package_row(p, 1)));
            } else {
                // The newest stands for the tool; what's installed shows too.
                let newest = packages[0];
                let mut paths = vec![newest.path.clone()];
                paths.extend(packages.iter().skip(1).filter(|p| self.local.contains_key(&p.path)).map(|p| p.path.clone()));
                let status = match self.status(&newest.path) {
                    Status::NotInstalled if paths.len() > 1 => Status::UpdateAvailable(newest.revision),
                    other => other,
                };
                rows.push(Row { name, api: String::new(), revision: newest.revision.to_string(), status: Some(status), packages: paths, depth: 0 });
            }
        }
        rows
    }
}

/// A versioned tool's name: Android Studio's, else the display name
/// without its version.
fn tool_name(prefix: &str, packages: &[&Package]) -> String {
    match prefix {
        "build-tools" => "Android SDK Build-Tools".into(),
        "ndk" => "NDK (Side by side)".into(),
        "cmake" => "CMake".into(),
        "cmdline-tools" => "Android SDK Command-line Tools".into(),
        _ if packages.len() == 1 => packages[0].display_name.clone(),
        _ => {
            let name = &packages[0].display_name;
            name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ').to_owned()
        }
    }
}

/// What Apply does: packages to download (new or updated) and to remove.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub install: Vec<Package>,
    pub uninstall: Vec<Package>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.install.is_empty() && self.uninstall.is_empty()
    }

    /// Licenses the downloads need that aren't accepted yet (`accepted`
    /// holds the ids whose text was accepted).
    pub fn licenses_to_accept<'a>(&self, repository: &'a Repository, accepted: &dyn Fn(&str, &str) -> bool) -> Vec<(&'a str, &'a str)> {
        let ids: BTreeSet<&String> = self.install.iter().filter_map(|p| p.license.as_ref()).collect();
        ids.into_iter().filter_map(|id| repository.licenses.get_key_value(id)).filter(|(id, text)| !accepted(id, text)).map(|(id, text)| (id.as_str(), text.as_str())).collect()
    }
}

/// The plan for the checked packages (`wanted`) and the updates asked for.
pub fn plan(catalog: &Catalog, wanted: &BTreeSet<String>, updates: &BTreeSet<String>) -> Plan {
    let mut plan = Plan::default();
    for path in wanted {
        let installed = catalog.local.contains_key(path);
        if !installed || updates.contains(path) {
            if let Some(remote) = catalog.remote.get(path).filter(|r| !installed || catalog.local.get(path).is_some_and(|l| r.revision > l.revision)) {
                plan.install.push(remote.clone());
            }
        }
    }
    plan.uninstall = catalog.local.values().filter(|p| !wanted.contains(&p.path)).cloned().collect();
    plan
}

/// Where packages are installed: the SDK folder.
pub trait SdkInstaller {
    /// Downloads, checks and unpacks the package (replacing an older
    /// revision); `progress` gets the bytes so far and in all.
    fn install(&self, package: &Package, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<(), String>;
    fn uninstall(&self, package: &Package) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sdk:sdk-repository xmlns:sdk="http://schemas.android.com/sdk/android/repo/repository2/03" xmlns:common="http://schemas.android.com/repository/android/common/02" xmlns:generic="http://schemas.android.com/repository/android/generic/02" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <license id="android-sdk-license" type="text">Terms &amp; conditions</license>
  <channel id="channel-0">stable</channel>
  <channel id="channel-1">beta</channel>
  <remotePackage path="platforms;android-34">
    <type-details xsi:type="sdk:platformDetailsType"><api-level>34</api-level><codename></codename><layoutlib api="15"/></type-details>
    <revision><major>3</major></revision>
    <display-name>Android SDK Platform 34</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-0"/>
    <archives><archive><complete><size>63048019</size><checksum type="sha1">ABCDEF</checksum><url>platform-34-ext7_r03.zip</url></complete></archive></archives>
  </remotePackage>
  <remotePackage path="sources;android-34">
    <type-details xsi:type="sdk:sourceDetailsType"><api-level>34</api-level></type-details>
    <revision><major>2</major></revision>
    <display-name>Sources for Android 34</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-0"/>
    <archives><archive><complete><size>10</size><checksum>1234</checksum><url>sources-34_r02.zip</url></complete></archive></archives>
  </remotePackage>
  <remotePackage path="platforms;android-33">
    <type-details xsi:type="sdk:platformDetailsType"><api-level>33</api-level></type-details>
    <revision><major>2</major></revision>
    <display-name>Android SDK Platform 33</display-name>
    <channelRef ref="channel-0"/>
    <archives><archive><complete><size>1</size><checksum type="sha1">aa</checksum><url>platform-33.zip</url></complete></archive></archives>
  </remotePackage>
  <remotePackage path="build-tools;35.0.0">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>35</major><minor>0</minor><micro>0</micro></revision>
    <display-name>Android SDK Build-Tools 35</display-name>
    <channelRef ref="channel-0"/>
    <archives>
      <archive><complete><size>5</size><checksum type="sha1">11</checksum><url>bt-linux.zip</url></complete><host-os>linux</host-os></archive>
      <archive><complete><size>6</size><checksum type="sha1">22</checksum><url>bt-mac.zip</url></complete><host-os>macosx</host-os></archive>
    </archives>
  </remotePackage>
  <remotePackage path="build-tools;34.0.0">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>34</major><minor>0</minor><micro>0</micro></revision>
    <display-name>Android SDK Build-Tools 34</display-name>
    <channelRef ref="channel-0"/>
    <archives><archive><complete><size>5</size><checksum type="sha1">33</checksum><url>bt34.zip</url></complete></archive></archives>
  </remotePackage>
  <remotePackage path="platform-tools">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>35</major><minor>0</minor><micro>2</micro></revision>
    <display-name>Android SDK Platform-Tools</display-name>
    <channelRef ref="channel-0"/>
    <archives><archive><complete><size>5</size><checksum type="sha1">44</checksum><url>pt.zip</url></complete></archive></archives>
  </remotePackage>
  <remotePackage path="platform-tools">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>36</major><minor>0</minor><micro>0</micro><preview>1</preview></revision>
    <display-name>Android SDK Platform-Tools</display-name>
    <channelRef ref="channel-1"/>
    <archives><archive><complete><size>5</size><checksum type="sha1">55</checksum><url>pt-beta.zip</url></complete></archive></archives>
  </remotePackage>
</sdk:sdk-repository>"#;

    fn local(path: &str, kind: Kind, api: Option<&str>, revision: Revision) -> Package {
        Package { path: path.into(), display_name: path.into(), revision, kind, api: api.map(str::to_owned), obsolete: false, license: None, archives: Vec::new(), local_xml: String::new() }
    }

    #[test]
    fn revisions() {
        assert_eq!(Revision::parse("35.0.1").unwrap().to_string(), "35.0.1");
        assert_eq!(Revision::parse("3").unwrap().to_string(), "3");
        assert!(Revision::parse("36.0.0 rc1").unwrap() < Revision::parse("36.0.0").unwrap());
        assert!(Revision::parse("35.0.2").unwrap() < Revision::parse("36.0.0 rc1").unwrap());
    }

    #[test]
    fn repository_keeps_stable_and_newest() {
        let repo = parse_repository(REPO);
        let paths: Vec<&str> = repo.packages.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["build-tools;34.0.0", "build-tools;35.0.0", "platform-tools", "platforms;android-33", "platforms;android-34", "sources;android-34"]);
        let tools = repo.packages.iter().find(|p| p.path == "platform-tools").unwrap();
        assert_eq!(tools.revision.to_string(), "35.0.2");
        let platform = repo.packages.iter().find(|p| p.path == "platforms;android-34").unwrap();
        assert_eq!((platform.kind, platform.api.as_deref(), platform.revision.to_string()), (Kind::Platform, Some("34"), "3".to_owned()));
        assert_eq!(platform.archives[0].sha1, "abcdef");
        assert_eq!(repo.licenses["android-sdk-license"], "Terms & conditions");
        assert!(!platform.local_xml.contains("archives") && platform.local_xml.contains("<layoutlib api=\"15\"/>"));
    }

    #[test]
    fn archive_for_this_machine() {
        let repo = parse_repository(REPO);
        let bt = repo.packages.iter().find(|p| p.path == "build-tools;35.0.0").unwrap();
        assert_eq!(archive_for(bt, "linux", "x86_64").unwrap().url, "bt-linux.zip");
        assert_eq!(archive_for(bt, "macos", "aarch64").unwrap().url, "bt-mac.zip");
        assert!(archive_for(bt, "windows", "x86_64").is_none());
    }

    #[test]
    fn written_package_xml_reads_back() {
        let repo = parse_repository(REPO);
        let platform = repo.packages.iter().find(|p| p.path == "platforms;android-34").unwrap();
        let xml = package_xml(platform, &repo);
        let back = parse_local(&xml).unwrap();
        assert_eq!((back.path.as_str(), back.kind, back.api.as_deref(), back.revision, back.display_name.as_str()), ("platforms;android-34", Kind::Platform, Some("34"), platform.revision, "Android SDK Platform 34"));
        assert!(xml.contains("<license id=\"android-sdk-license\" type=\"text\">Terms &amp; conditions</license>"));
    }

    #[test]
    fn platform_rows_summary_and_details() {
        let repo = parse_repository(REPO);
        let installed = [local("platforms;android-34", Kind::Platform, Some("34"), Revision { major: 2, precision: 1, ..Default::default() })];
        let catalog = Catalog::new(&repo.packages, &installed);
        let rows = catalog.platform_rows(false, true);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Android 14.0 (\"UpsideDownCake\")");
        assert_eq!(rows[0].status, Some(Status::UpdateAvailable(repo.packages.iter().find(|p| p.path == "platforms;android-34").unwrap().revision)));
        assert_eq!(rows[0].packages, ["platforms;android-34", "sources;android-34"]);
        assert_eq!(rows[1].status, Some(Status::NotInstalled));
        let details = catalog.platform_rows(true, true);
        let names: Vec<(&str, u8)> = details.iter().map(|r| (r.name.as_str(), r.depth)).collect();
        assert_eq!(names, [("Android 14.0 (\"UpsideDownCake\")", 0), ("platforms;android-34", 1), ("Sources for Android 34", 1), ("Android 13.0 (\"Tiramisu\")", 0), ("Android SDK Platform 33", 1)]);
    }

    #[test]
    fn tool_rows_group_versions() {
        let repo = parse_repository(REPO);
        let installed = [local("build-tools;34.0.0", Kind::Other, None, Revision::new(34, 0, 0, None))];
        let catalog = Catalog::new(&repo.packages, &installed);
        let rows = catalog.tool_rows(false, true);
        let summary: Vec<(&str, &str)> = rows.iter().map(|r| (r.name.as_str(), r.revision.as_str())).collect();
        assert_eq!(summary, [("Android SDK Build-Tools", "35.0.0"), ("Android SDK Platform-Tools", "35.0.2")]);
        assert_eq!(rows[0].packages, ["build-tools;35.0.0", "build-tools;34.0.0"]);
        assert!(matches!(rows[0].status, Some(Status::UpdateAvailable(_))));
        let details = catalog.tool_rows(true, true);
        assert_eq!(details.iter().map(|r| r.depth).collect::<Vec<_>>(), [0, 1, 1, 0]);
    }

    #[test]
    fn plan_installs_removes_and_updates() {
        let repo = parse_repository(REPO);
        let installed = [
            local("platforms;android-34", Kind::Platform, Some("34"), Revision { major: 2, precision: 1, ..Default::default() }),
            local("build-tools;34.0.0", Kind::Other, None, Revision::new(34, 0, 0, None)),
        ];
        let catalog = Catalog::new(&repo.packages, &installed);
        let wanted: BTreeSet<String> = ["platforms;android-34", "platform-tools"].map(String::from).into();
        let p = plan(&catalog, &wanted, &BTreeSet::new());
        assert_eq!(p.install.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["platform-tools"]);
        assert_eq!(p.uninstall.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["build-tools;34.0.0"]);
        let p = plan(&catalog, &wanted, &["platforms;android-34".to_owned()].into());
        assert_eq!(p.install.len(), 2);
        let licenses = p.licenses_to_accept(&repo, &|_, _| false);
        assert_eq!(licenses, [("android-sdk-license", "Terms & conditions")]);
        assert!(p.licenses_to_accept(&repo, &|_, _| true).is_empty());
    }
}
