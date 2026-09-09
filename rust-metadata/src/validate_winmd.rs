use std::path::PathBuf;

use sf_winmd_gen::validation;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    let migration = first.as_deref() == Some(std::ffi::OsStr::new("--migration-baseline"));
    let baseline = if migration { args.next() } else { first }.map(PathBuf::from);
    let candidate = args.next().map(PathBuf::from);

    if baseline.is_none() || candidate.is_none() || args.next().is_some() {
        eprintln!(
            "usage: validate_winmd [--migration-baseline] <baseline.winmd> <candidate.winmd>"
        );
        std::process::exit(2);
    }

    let baseline = baseline.unwrap();
    let candidate = candidate.unwrap();
    let baseline = validation::load(&baseline).unwrap_or_else(|error| {
        eprintln!("baseline: {error}");
        std::process::exit(2);
    });
    let candidate = validation::load(&candidate).unwrap_or_else(|error| {
        eprintln!("candidate: {error}");
        std::process::exit(2);
    });

    let differences = if migration {
        validation::compare_migration(&baseline, &candidate)
    } else {
        validation::compare(&baseline, &candidate)
    };
    if differences.is_empty() {
        println!(
            "winmd validation passed ({} baseline types, {} candidate types)",
            validation::type_count(&baseline),
            validation::type_count(&candidate)
        );
        return;
    }

    eprintln!("winmd structural differences:");
    for difference in differences {
        eprintln!("  - {difference}");
    }
    std::process::exit(1);
}
