use colored::Colorize;
use std::io::{self, IsTerminal};

pub fn configure(no_color: bool) {
    if no_color || std::env::var_os("NO_COLOR").is_some() || !io::stdout().is_terminal() {
        colored::control::set_override(false);
    }
}

// API-controlled names must not inject terminal control sequences or new lines.
pub fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(256)
        .collect()
}

pub fn heading(title: &str) {
    println!(
        "\n  {} {}",
        "PENGWALLET".cyan().bold(),
        format!("/ {title}").dimmed()
    );
    println!("  {}", "────────────────────────────────────────".dimmed());
}

pub fn field(label: &str, value: impl AsRef<str>) {
    println!("  {:<12} {}", label.dimmed(), clean(value.as_ref()));
}

pub fn success(message: &str) {
    println!("\n  {} {}", "✓".green().bold(), message.green().bold());
}

pub fn hint(message: &str) {
    println!("\n  {}\n", message.dimmed());
}

pub fn amount(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(digit);
    }
    output
}

pub fn balance(symbol: &str, value: u64) {
    println!("  {:>20}  {}", amount(value).bold(), clean(symbol).cyan());
}

pub fn progress(message: &str) {
    eprintln!("  · {}", clean(message));
}

pub fn error(message: &str) {
    eprintln!("\n  Error: {}\n", clean(message));
}
