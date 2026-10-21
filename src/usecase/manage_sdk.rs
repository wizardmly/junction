//! The SDK Manager's Apply: removes the unchecked packages, then downloads
//! the checked and updated ones one by one, as Android Studio's SDK
//! Component Installer, reporting each step.

use crate::domain::sdk_packages::{Plan, SdkInstaller};

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Uninstalling(String),
    /// A package's name, the bytes so far and in all.
    Downloading(String, u64, u64),
    Installed(String),
    Uninstalled(String),
    Failed(String, String),
}

/// Runs the plan; whether every package went through. A cancelled run
/// stops before the next package.
pub fn apply(installer: &dyn SdkInstaller, plan: &Plan, report: &mut dyn FnMut(Step), cancelled: &dyn Fn() -> bool) -> bool {
    let mut ok = true;
    for package in &plan.uninstall {
        if cancelled() {
            return false;
        }
        report(Step::Uninstalling(package.display_name.clone()));
        match installer.uninstall(package) {
            Ok(()) => report(Step::Uninstalled(package.display_name.clone())),
            Err(e) => {
                ok = false;
                report(Step::Failed(package.display_name.clone(), e));
            }
        }
    }
    for package in &plan.install {
        if cancelled() {
            return false;
        }
        let name = package.display_name.clone();
        report(Step::Downloading(name.clone(), 0, package.archives.first().map_or(0, |a| a.size)));
        let result = installer.install(package, &mut |done, total| report(Step::Downloading(name.clone(), done, total)), cancelled);
        match result {
            Ok(()) => report(Step::Installed(name)),
            Err(e) => {
                ok = false;
                report(Step::Failed(name, e));
            }
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sdk_packages::{Kind, Package, Revision};
    use std::cell::RefCell;

    struct Fake(RefCell<Vec<String>>);

    impl SdkInstaller for Fake {
        fn install(&self, package: &Package, progress: &mut dyn FnMut(u64, u64), _: &dyn Fn() -> bool) -> Result<(), String> {
            progress(5, 10);
            self.0.borrow_mut().push(format!("+{}", package.path));
            if package.path == "bad" { Err("checksum".into()) } else { Ok(()) }
        }

        fn uninstall(&self, package: &Package) -> Result<(), String> {
            self.0.borrow_mut().push(format!("-{}", package.path));
            Ok(())
        }
    }

    fn package(path: &str) -> Package {
        Package { path: path.into(), display_name: path.into(), revision: Revision::default(), kind: Kind::Other, api: None, obsolete: false, license: None, archives: Vec::new(), local_xml: String::new() }
    }

    #[test]
    fn uninstalls_then_installs_and_reports() {
        let fake = Fake(RefCell::default());
        let plan = Plan { install: vec![package("a"), package("bad")], uninstall: vec![package("old")] };
        let mut steps = Vec::new();
        let ok = apply(&fake, &plan, &mut |s| steps.push(s), &|| false);
        assert!(!ok);
        assert_eq!(*fake.0.borrow(), ["-old", "+a", "+bad"]);
        assert!(steps.contains(&Step::Downloading("a".into(), 5, 10)));
        assert_eq!(steps.last(), Some(&Step::Failed("bad".into(), "checksum".into())));
    }

    #[test]
    fn cancelled_stops() {
        let fake = Fake(RefCell::default());
        let plan = Plan { install: vec![package("a")], uninstall: Vec::new() };
        assert!(!apply(&fake, &plan, &mut |_| {}, &|| true));
        assert!(fake.0.borrow().is_empty());
    }
}
