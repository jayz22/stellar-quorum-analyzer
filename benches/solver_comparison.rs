use batsat::{dimacs::parse, lbool, BasicCallbacks, Solver as BatSatSolver, SolverInterface};
use prettytable::{format, Cell, Row, Table};
use screwsat::solver::Solver as ScrewSatSolver;
use splr::{SolveIF, Solver as SplrSolver};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::File,
    io::{BufReader, Write},
    path::Path,
    time::Instant,
};
use varisat::Solver as VariSatSolver;

const FILE_PATH: &str = "tests/test_data/random";

#[derive(Default, Debug)]
pub(crate) enum Status {
    #[default]
    UNSAT,
    SAT,
}

impl ToString for Status {
    fn to_string(&self) -> String {
        match self {
            Status::UNSAT => "UNSAT",
            Status::SAT => "SAT",
        }
        .to_string()
    }
}

#[derive(Default, Debug)]
pub(crate) struct MeasurementResult {
    pub solver_name: String,
    pub setup_time: u64,
    pub solve_time: u64,
    pub status: Status,
}

#[derive(Default)]
struct ScrewSat {
    solver: Option<ScrewSatSolver>,
}

#[derive(Default)]
struct VariSat<'a> {
    solver: Option<VariSatSolver<'a>>,
}

#[derive(Default)]
struct Splr {
    solver: Option<SplrSolver>,
}

#[derive(Default)]
struct BatSat {
    solver: Option<BatSatSolver<BasicCallbacks>>,
}

fn measure_execution<T, F: FnOnce() -> T>(f: F) -> (u64, T) {
    let start_time = Instant::now();
    let result = f();
    let stop_time = Instant::now();
    let time_usecs = stop_time.duration_since(start_time).as_micros() as u64;
    (time_usecs, result)
}

pub(crate) trait CNFSolverMeasurement {
    fn measured_setup(&mut self, path: &Path) -> std::io::Result<u64>;
    fn measured_solve(&mut self) -> std::io::Result<(u64, Status)>;
    fn solver_name() -> String;
}

impl CNFSolverMeasurement for ScrewSat {
    fn measured_setup(&mut self, path: &Path) -> std::io::Result<u64> {
        let input = File::open(path).unwrap();
        let (time_usecs, ()) = measure_execution(|| {
            let cnf = screwsat::util::parse_cnf(input).unwrap();
            self.solver = Some(ScrewSatSolver::new(cnf.var_num.unwrap(), &cnf.clauses));
        });
        Ok(time_usecs)
    }

    fn measured_solve(&mut self) -> std::io::Result<(u64, Status)> {
        if let Some(sat) = &mut self.solver {
            let (time_usecs, status) = measure_execution(|| match sat.solve(None) {
                screwsat::solver::Status::Sat => Status::SAT,
                screwsat::solver::Status::Unsat => Status::UNSAT,
                screwsat::solver::Status::Indeterminate => panic!("solver stopped searching"),
            });
            Ok((time_usecs, status))
        } else {
            panic!("solver has not been setup")
        }
    }

    fn solver_name() -> String {
        "ScrewSat".to_string()
    }
}

impl<'a> CNFSolverMeasurement for VariSat<'a> {
    fn measured_setup(&mut self, path: &Path) -> std::io::Result<u64> {
        let file = File::open(path).unwrap();
        let reader = BufReader::new(file);
        let (time_usecs, ()) = measure_execution(|| {
            let mut solver = VariSatSolver::new();
            solver.add_dimacs_cnf(reader).unwrap();
            self.solver = Some(solver);
        });
        Ok(time_usecs)
    }

    fn measured_solve(&mut self) -> std::io::Result<(u64, Status)> {
        if let Some(sat) = &mut self.solver {
            let (time_usecs, status) = measure_execution(|| match sat.solve().unwrap() {
                true => Status::SAT,
                false => Status::UNSAT,
            });
            Ok((time_usecs, status))
        } else {
            panic!("solver has not been setup")
        }
    }

    fn solver_name() -> String {
        "VariSat".to_string()
    }
}

impl CNFSolverMeasurement for Splr {
    fn measured_setup(&mut self, path: &Path) -> std::io::Result<u64> {
        let (time_usecs, ()) = measure_execution(|| {
            self.solver = Some(SplrSolver::try_from(path).unwrap());
        });
        Ok(time_usecs)
    }

    fn measured_solve(&mut self) -> std::io::Result<(u64, Status)> {
        if let Some(sat) = &mut self.solver {
            let (time_usecs, status) = measure_execution(|| match sat.solve().unwrap() {
                splr::Certificate::SAT(_) => Status::SAT,
                splr::Certificate::UNSAT => Status::UNSAT,
            });
            Ok((time_usecs, status))
        } else {
            panic!("solver has not been setup")
        }
    }

    fn solver_name() -> String {
        "Splr".to_string()
    }
}

impl CNFSolverMeasurement for BatSat {
    fn measured_setup(&mut self, path: &Path) -> std::io::Result<u64> {
        let file = File::open(path).unwrap();
        let mut reader = BufReader::new(file);
        let (time_usecs, ()) = measure_execution(|| {
            let mut solver = BatSatSolver::new(Default::default(), Default::default());
            parse(&mut reader, &mut solver, true, false).unwrap();
            self.solver = Some(solver);
        });
        Ok(time_usecs)
    }

    fn measured_solve(&mut self) -> std::io::Result<(u64, Status)> {
        if let Some(sat) = &mut self.solver {
            let (time_usecs, status) = measure_execution(|| {
                let res = sat.solve_limited(&[]);
                if res == lbool::TRUE {
                    Status::SAT
                } else if res == lbool::FALSE {
                    Status::UNSAT
                } else {
                    panic!("Solver returned UNDEF")
                }
            });
            Ok((time_usecs, status))
        } else {
            panic!("solver has not been setup")
        }
    }

    fn solver_name() -> String {
        "BatSat".to_string()
    }
}

fn for_each_dimacs_file<C: CNFSolverMeasurement>(
    path: &str,
    solver: &mut C,
    results: &mut BTreeMap<OsString, Vec<MeasurementResult>>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let file_stem = path.file_stem().unwrap().to_os_string();

        // Check if the file has a .dimacs extension
        if let Some(extension) = path.extension() {
            if extension == "dimacs" {
                // Get the file name without the extension
                let mut res = MeasurementResult {
                    solver_name: C::solver_name(),
                    ..Default::default()
                };
                res.setup_time = C::measured_setup(solver, path.as_path()).unwrap();
                (res.solve_time, res.status) = C::measured_solve(solver).unwrap();

                results.entry(file_stem).or_insert_with(Vec::new).push(res);
            }
        }
    }

    Ok(())
}

fn output_results(results: &BTreeMap<OsString, Vec<MeasurementResult>>) -> std::io::Result<()> {
    let mut table = Table::new();
    table.set_format(*format::consts::FORMAT_CLEAN);
    table.add_row(Row::new(vec![
        Cell::new("file_name"),
        Cell::new("solver_name"),
        Cell::new("setup_time (usecs)"),
        Cell::new("solve_time (usecs)"),
        Cell::new("status"),
    ]));
    for (file_name, measurements) in results {
        for measurement in measurements {
            table.add_row(Row::new(vec![
                Cell::new(&file_name.to_string_lossy()),
                Cell::new(&measurement.solver_name),
                Cell::new(&measurement.setup_time.to_string()),
                Cell::new(&measurement.solve_time.to_string()),
                Cell::new(&measurement.status.to_string()),
            ]));
        }
    }

    table.printstd();
    let mut result_table = File::create("results_table.txt")?;
    write!(result_table, "{}", table)
}

fn main() -> std::io::Result<()> {
    assert!(
        Path::new(FILE_PATH).is_dir(),
        "Directory not found: {}",
        FILE_PATH
    );

    let mut results: BTreeMap<OsString, Vec<MeasurementResult>> = Default::default();

    for_each_dimacs_file(FILE_PATH, &mut ScrewSat::default(), &mut results)?;
    for_each_dimacs_file::<VariSat>(FILE_PATH, &mut VariSat::default(), &mut results)?;
    for_each_dimacs_file::<Splr>(FILE_PATH, &mut Splr::default(), &mut results)?;
    for_each_dimacs_file::<BatSat>(FILE_PATH, &mut BatSat::default(), &mut results)?;

    output_results(&results)
}
