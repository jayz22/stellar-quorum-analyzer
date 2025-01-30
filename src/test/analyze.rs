use crate::{FbasAnalyzer, SolveStatus};
use batsat::callbacks::{AsyncInterrupt, Basic};
use std::{io::BufRead, str::FromStr};

#[test]
fn test_solver_interrupt() -> Result<(), Box<dyn std::error::Error>> {
    let json_file = std::path::PathBuf::from(
        "./tests/test_data/random/almost_symmetric_network_16_orgs_delete_prob_factor_1.json",
    );
    let cb = AsyncInterrupt::default();
    let handle = cb.get_handle();
    let mut solver = FbasAnalyzer::from_json_path(json_file.as_os_str().to_str().unwrap(), cb)?;

    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_micros(100));
        handle.interrupt_async();
    });

    let res = solver.solve();
    assert_eq!(res, SolveStatus::UNKNOWN);
    Ok(())
}

#[test]
fn test() -> std::io::Result<()> {
    let mut test_cases = vec![];
    for entry in std::fs::read_dir("./tests/test_data/")? {
        let path = entry?.path();
        if let Some(extension) = path.extension() {
            if extension == "json" {
                let case = path.file_stem().unwrap().to_os_string();
                test_cases.push(case);
                let mut solver = FbasAnalyzer::from_json_path(
                    path.as_os_str().to_str().unwrap(),
                    Basic::default(),
                )
                .unwrap();
                let res = solver.solve();
                println!("{:?}", res);
            }
        }
    }
    Ok(())
}

#[test]
fn test_random_data() -> std::io::Result<()> {
    let mut test_cases = vec![];
    let dir_path = std::ffi::OsString::from_str("./tests/test_data/random/").unwrap();
    for entry in std::fs::read_dir("./tests/test_data/random/")? {
        let path = entry?.path();
        if let Some(extension) = path.extension() {
            if extension == "dimacs" {
                let case = path.file_stem().unwrap().to_os_string();
                test_cases.push(case);
            }
        }
    }

    for case in test_cases {
        let mut json_file = dir_path.clone();
        json_file.push(case.clone());
        json_file.push(".json");

        let mut dimacs_file = dir_path.clone();
        dimacs_file.push(case.clone());
        dimacs_file.push(".dimacs");

        let mut solver =
            FbasAnalyzer::from_json_path(json_file.as_os_str().to_str().unwrap(), Basic::default())
                .unwrap();
        let res = solver.solve();
        {
            // Open and read the file line by line
            let file = std::fs::File::open(dimacs_file).expect("Failed to open the DIMACS file");
            let reader = std::io::BufReader::new(file);

            // Look for the result comment line
            let mut expected = false;
            for line in reader.lines() {
                let line = line.expect("Failed to read line");
                if line.starts_with("c") {
                    if line.contains("UNSATISFIABLE") {
                        expected = false;
                        break;
                    } else if line.contains("SATISFIABLE") {
                        expected = true;
                        let (qa, qb) = solver.get_potential_split().unwrap();
                        println!("quorum a: {:?}, quorum b: {:?}", qa, qb);
                        break;
                    }
                }
            }
            let is_sat = matches!(res, SolveStatus::SAT(_));
            assert_eq!(is_sat, expected);
        }
    }
    Ok(())
}
