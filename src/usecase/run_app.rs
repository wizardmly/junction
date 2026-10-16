//! Run 'app', as Android Studio runs an Android App configuration: assemble
//! the module's selected variant, install its APKs on the device, stop the
//! running app, start its launcher activity and report its process. The
//! Run console gets Android Studio's lines; the build goes to the Build
//! window.

use std::time::{Duration, Instant};

use crate::domain::run::{activity_component, duration_label, start_command, BuildResult, DeviceBridge, LaunchTarget, RunConfiguration, TaskRunner};

/// A line of the Run console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleLine {
    Info(String),
    Command(String),
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// The app runs: its process, when found.
    Running { application_id: String, pid: Option<u32> },
    BuildFailed,
    Cancelled,
    Failed,
}

/// Make Project / Make Module: the tasks assembling every given module's
/// selected variant.
pub fn make(runner: &dyn TaskRunner, build_root: &std::path::Path, tasks: &[String], line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> BuildResult {
    runner.run_tasks(build_root, tasks, line, cancelled)
}

pub struct Target<'a> {
    pub serial: &'a str,
    /// "Google Pixel 9a".
    pub name: &'a str,
    /// Debug 'app': the app waits for the debugger.
    pub debug: bool,
}

/// Builds and launches. `build_line` gets Gradle's output, `console` the
/// Run console's lines.
pub fn run(
    runner: &dyn TaskRunner,
    device: &dyn DeviceBridge,
    config: &RunConfiguration,
    variant: &str,
    target: Target,
    build_line: &mut dyn FnMut(&str),
    console: &mut dyn FnMut(ConsoleLine),
    cancelled: &dyn Fn() -> bool,
) -> (RunOutcome, BuildResult) {
    let build = runner.run_tasks(&config.build_root, &[config.module.assemble_task(variant)], build_line, cancelled);
    if build.cancelled || cancelled() {
        return (RunOutcome::Cancelled, build);
    }
    if !build.succeeded {
        return (RunOutcome::BuildFailed, build);
    }
    let Some(output) = runner.apk_output(&config.module, variant) else {
        console(ConsoleLine::Error(format!("Error running '{}': The APK file for the {variant} variant was not found. Was it built?", config.name)));
        return (RunOutcome::Failed, build);
    };
    // Deploy: "Nothing" leaves what's installed.
    if config.options.deploy {
        match device.install(target.serial, &output.apks) {
            Ok(took) => console(ConsoleLine::Info(format!("Install successfully finished in {}.", duration_label(took)))),
            Err(reason) => {
                console(ConsoleLine::Error("Installation did not succeed.".into()));
                console(ConsoleLine::Error(format!("The application could not be installed: {reason}")));
                return (RunOutcome::Failed, build);
            }
        }
    }
    if cancelled() {
        return (RunOutcome::Cancelled, build);
    }
    let component = match &config.options.launch {
        // Launch: "Nothing".
        LaunchTarget::Nothing => return (RunOutcome::Running { application_id: output.application_id, pid: None }, build),
        LaunchTarget::Activity(name) => activity_component(&output.application_id, name),
        LaunchTarget::DefaultActivity => match device.launcher_activity(target.serial, &output.application_id) {
            Some(component) => component,
            None => {
                console(ConsoleLine::Error("Could not identify launch activity: Default Activity not found".into()));
                console(ConsoleLine::Error("Error while Launching activity".into()));
                return (RunOutcome::Failed, build);
            }
        },
    };
    device.force_stop(target.serial, &output.application_id);
    console(ConsoleLine::Command(format!("$ adb shell {}", start_command(&component, target.debug, &config.options.flags))));
    if let Err(reason) = device.start(target.serial, &component, target.debug, &config.options.flags) {
        console(ConsoleLine::Error(reason));
        console(ConsoleLine::Error("Error while Launching activity".into()));
        return (RunOutcome::Failed, build);
    }
    // The process shows up once the activity starts.
    let mut pid = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while pid.is_none() && Instant::now() < deadline && !cancelled() {
        pid = device.pid(target.serial, &output.application_id);
        if pid.is_none() {
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    match pid {
        Some(_) if target.debug => console(ConsoleLine::Info(format!("Waiting for application to come online: {}", output.application_id))),
        Some(pid) => console(ConsoleLine::Info(format!("Connected to process {pid} on device '{}'.", target.name))),
        None => console(ConsoleLine::Info(format!("The process of {} didn't start on '{}'.", output.application_id, target.name))),
    }
    (RunOutcome::Running { application_id: output.application_id, pid }, build)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::run::{ApkOutput, ModuleInfo, ModuleType};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    struct Gradle {
        ok: bool,
        tasks: RefCell<Vec<String>>,
    }
    impl TaskRunner for Gradle {
        fn run_tasks(&self, _: &Path, tasks: &[String], line: &mut dyn FnMut(&str), _: &dyn Fn() -> bool) -> BuildResult {
            self.tasks.borrow_mut().extend(tasks.iter().cloned());
            line("BUILD SUCCESSFUL in 1s");
            BuildResult { succeeded: self.ok, ..Default::default() }
        }
        fn apk_output(&self, _: &ModuleInfo, _: &str) -> Option<ApkOutput> {
            Some(ApkOutput { application_id: "com.example".into(), apks: vec![PathBuf::from("/p/app.apk")] })
        }
    }

    struct Phone(RefCell<Vec<String>>);
    impl DeviceBridge for Phone {
        fn install(&self, _: &str, apks: &[PathBuf]) -> Result<Duration, String> {
            self.0.borrow_mut().push(format!("install {}", apks.len()));
            Ok(Duration::from_millis(1500))
        }
        fn force_stop(&self, _: &str, id: &str) {
            self.0.borrow_mut().push(format!("stop {id}"));
        }
        fn launcher_activity(&self, _: &str, id: &str) -> Option<String> {
            Some(format!("{id}/.MainActivity"))
        }
        fn start(&self, _: &str, component: &str, debug: bool, flags: &str) -> Result<(), String> {
            self.0.borrow_mut().push(format!("start {component}{}{}", if debug { " -D" } else { "" }, if flags.is_empty() { String::new() } else { format!(" {flags}") }));
            Ok(())
        }
        fn pid(&self, _: &str, _: &str) -> Option<u32> {
            Some(4242)
        }
    }

    fn config() -> RunConfiguration {
        let module = ModuleInfo { path: ":app".into(), dir: "/p/app".into(), kind: ModuleType::Application, variants: vec!["debug".into()] };
        RunConfiguration { name: "app".into(), build_root: "/p".into(), module, options: Default::default() }
    }

    #[test]
    fn builds_installs_and_starts() {
        let (gradle, phone) = (Gradle { ok: true, tasks: RefCell::default() }, Phone(RefCell::default()));
        let mut lines = Vec::new();
        let (outcome, _) = run(&gradle, &phone, &config(), "debug", Target { serial: "s", name: "Pixel", debug: false }, &mut |_| {}, &mut |l| lines.push(l), &|| false);
        assert_eq!(outcome, RunOutcome::Running { application_id: "com.example".into(), pid: Some(4242) });
        assert_eq!(gradle.tasks.borrow().as_slice(), [":app:assembleDebug"]);
        assert_eq!(phone.0.borrow().as_slice(), ["install 1", "stop com.example", "start com.example/.MainActivity"]);
        assert_eq!(lines[0], ConsoleLine::Info("Install successfully finished in 1 s 500 ms.".into()));
        assert!(matches!(&lines[1], ConsoleLine::Command(c) if c.contains("am start -n \"com.example/.MainActivity\"")));
        assert_eq!(lines[2], ConsoleLine::Info("Connected to process 4242 on device 'Pixel'.".into()));
    }

    #[test]
    fn a_failed_build_installs_nothing() {
        let (gradle, phone) = (Gradle { ok: false, tasks: RefCell::default() }, Phone(RefCell::default()));
        let (outcome, _) = run(&gradle, &phone, &config(), "debug", Target { serial: "s", name: "Pixel", debug: false }, &mut |_| {}, &mut |_| {}, &|| false);
        assert_eq!(outcome, RunOutcome::BuildFailed);
        assert!(phone.0.borrow().is_empty());
    }

    #[test]
    fn debug_waits_for_the_debugger() {
        let (gradle, phone) = (Gradle { ok: true, tasks: RefCell::default() }, Phone(RefCell::default()));
        let mut lines = Vec::new();
        let (outcome, _) = run(&gradle, &phone, &config(), "debug", Target { serial: "s", name: "Pixel", debug: true }, &mut |_| {}, &mut |l| lines.push(l), &|| false);
        assert_eq!(outcome, RunOutcome::Running { application_id: "com.example".into(), pid: Some(4242) });
        assert_eq!(phone.0.borrow().last().map(String::as_str), Some("start com.example/.MainActivity -D"));
        assert!(matches!(&lines[1], ConsoleLine::Command(c) if c.ends_with("LAUNCHER -D")));
        assert_eq!(lines[2], ConsoleLine::Info("Waiting for application to come online: com.example".into()));
    }

    #[test]
    fn follows_the_launch_options() {
        use crate::domain::run::{LaunchOptions, LaunchTarget};
        // A specified activity with launch flags.
        let (gradle, phone) = (Gradle { ok: true, tasks: RefCell::default() }, Phone(RefCell::default()));
        let mut config = config();
        config.options = LaunchOptions { deploy: true, launch: LaunchTarget::Activity(".SettingsActivity".into()), flags: "--ez demo true".into() };
        let mut lines = Vec::new();
        run(&gradle, &phone, &config, "debug", Target { serial: "s", name: "Pixel", debug: false }, &mut |_| {}, &mut |l| lines.push(l), &|| false);
        assert_eq!(phone.0.borrow().as_slice(), ["install 1", "stop com.example", "start com.example/.SettingsActivity --ez demo true"]);
        assert!(matches!(&lines[1], ConsoleLine::Command(c) if c.ends_with("LAUNCHER --ez demo true")));
        // Deploy and Launch "Nothing": built only.
        let phone = Phone(RefCell::default());
        config.options = LaunchOptions { deploy: false, launch: LaunchTarget::Nothing, flags: String::new() };
        let (outcome, _) = run(&gradle, &phone, &config, "debug", Target { serial: "s", name: "Pixel", debug: false }, &mut |_| {}, &mut |_| {}, &|| false);
        assert_eq!(outcome, RunOutcome::Running { application_id: "com.example".into(), pid: None });
        assert!(phone.0.borrow().is_empty());
    }
}
