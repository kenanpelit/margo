use colored::*;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
// Note: io and Write were imported but are unused - keeping commented for reference
// use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Create a dot-style spinner for indeterminate operations
pub fn create_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    spinner.set_message(message.to_string());
    spinner
}

/// Creates a simple text-based progress display for module processing
/// No spinners or progress bars - just clean text output with elapsed time
pub struct DetailedProgress {
    total_packages: u64,
    completed: u64,
    failed: Vec<String>,
    package_start: Instant,
    current_package: String,
    timer_active: Arc<AtomicBool>,
    timer_thread: Option<std::thread::JoinHandle<()>>,
}

impl DetailedProgress {
    pub fn new(
        module_name: &str,
        module_num: usize,
        total_modules: usize,
        total_packages: u64,
    ) -> Self {
        // Header separator
        let header = format!(
            "\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n Module: {} ({}/{})\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
            module_name.cyan().bold(),
            module_num,
            total_modules
        );
        println!("{}", header);

        Self {
            total_packages,
            completed: 0,
            failed: Vec::new(),
            package_start: Instant::now(),
            current_package: String::new(),
            timer_active: Arc::new(AtomicBool::new(false)),
            timer_thread: None,
        }
    }

    pub fn set_current_package(&mut self, package_name: &str) {
        // Stop any existing timer thread
        self.stop_timer();

        self.package_start = Instant::now();
        self.current_package = package_name.to_string();

        // Print the package being installed on its own line
        println!(
            " [{}/{}] Installing {}...",
            self.completed + 1,
            self.total_packages,
            package_name.cyan()
        );

        // Start the timer update thread (but don't update the display since we're on separate lines)
        self.start_timer();
    }

    fn start_timer(&mut self) {
        self.timer_active.store(true, Ordering::SeqCst);
        let timer_active = Arc::clone(&self.timer_active);
        let _package_name = self.current_package.clone();
        let _completed = self.completed;
        let _total_packages = self.total_packages;
        let start_time = Instant::now();

        let handle = std::thread::spawn(move || {
            while timer_active.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(500));

                if timer_active.load(Ordering::SeqCst) {
                    let _elapsed = start_time.elapsed().as_secs();
                    // Don't update the display since we want to keep the sudo prompt clean
                    // Just track time silently
                }
            }
        });

        self.timer_thread = Some(handle);
    }

    fn stop_timer(&mut self) {
        if self.timer_thread.is_some() {
            self.timer_active.store(false, Ordering::SeqCst);
            if let Some(handle) = self.timer_thread.take() {
                let _ = handle.join();
            }
        }
    }

    pub fn package_completed(&mut self, package_name: &str, _duration_secs: f64, success: bool) {
        // Stop the timer thread first
        self.stop_timer();

        let elapsed = self.package_start.elapsed().as_secs();
        self.completed += 1;

        // Print the final status on a new line
        if success {
            println!(
                " [{}/{}] {} {} ({}s)",
                self.completed,
                self.total_packages,
                "✓".green(),
                package_name.green(),
                elapsed
            );
        } else {
            println!(
                " [{}/{}] {} {} ({}s)",
                self.completed,
                self.total_packages,
                "✗".red(),
                package_name.red(),
                elapsed
            );
            self.failed.push(package_name.to_string());
        }
    }

    pub fn finish(mut self, _module_name: &str) {
        // Ensure timer is stopped
        self.stop_timer();

        // Show failures if any occurred
        if !self.failed.is_empty() {
            println!("\n {} Failed packages:", "✗".red());
            for pkg in &self.failed {
                println!("   {} {}", "✗".red(), pkg);
            }
        }

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    }
}

impl Drop for DetailedProgress {
    fn drop(&mut self) {
        // Ensure the timer thread is stopped when the struct is dropped
        self.stop_timer();
    }
}
