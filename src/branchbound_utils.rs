use crate::branchbound::BBSolver;
use crate::qubo::Qubo;
use clarabel::algebra::CscMatrix;
use ndarray::Array1;
use smolprng::{JsfLarge, PRNG};
use sprs::CsMat;
use std::collections::HashMap;

/// Bare bones implementation of B&B. Currently requires the QUBO to be symmetrical and convex.
/// Currently, the deterministic solver is solved via Clarabel.rs.

/// Struct the describes the branch and bound tree nodes
#[derive(Clone)]
pub struct QuboBBNode {
    pub lower_bound: f64,
    pub solution: Array1<f64>,
    pub fixed_variables: HashMap<usize, f64>,
}

/// Options for the B&B solver for run time
pub struct SolverOptions {
    pub fixed_variables: HashMap<usize, f64>,
    pub branch_strategy: BranchStrategy,
    pub max_time: f64,
    pub seed: usize,
}

pub enum BranchStrategy {
    FirstNotFixed,
    MostViolated,
    Random,
    WorstApproximation,
    BestApproximation,
}

pub fn first_not_fixed(solver: &BBSolver, node: &QuboBBNode) -> usize {
    // scan through the variables and find the first one that is not fixed
    for i in 0..solver.qubo.num_x() {
        if !node.fixed_variables.contains_key(&i) {
            return i;
        }
    }
    panic!("No variable to branch on");
}

pub fn most_violated(solver: &BBSolver, node: &QuboBBNode) -> usize {
    let mut most_violated = 1.0;
    let mut index_most_violated = 0;

    for i in 0..solver.qubo.num_x() {
        if !node.fixed_variables.contains_key(&i) {
            let violation = (node.solution[i] - 0.5).abs();

            if violation <= most_violated {
                most_violated = violation;
                index_most_violated = i;
            }
        }
    }

    index_most_violated
}

pub fn random(solver: &BBSolver, node: &QuboBBNode) -> usize {
    // generate a prng
    let mut prng = PRNG {
        generator: JsfLarge::from(solver.options.seed as u64 + solver.nodes_visited as u64),
    };

    // generate a random index in the list of variables
    let index = (prng.gen_u64() % solver.qubo.num_x() as u64) as usize;

    // scan thru the variables and find the first one that is not fixed starting at the random point
    for i in index..solver.qubo.num_x() {
        if !node.fixed_variables.contains_key(&i) {
            return i;
        }
    }

    // scan through the variables and find the first one that is not fixed starting at the beginning
    for i in 0..index {
        if !node.fixed_variables.contains_key(&i) {
            return i;
        }
    }

    panic!("No Variable to branch on")
}

/// Branches on the variable that has an estimated worst result, pushing up the lower bound as fast as possible
pub fn worst_approximation(solver: &BBSolver, node: &QuboBBNode) -> usize {
    let (zero_flip, one_flip) = compute_strong_branch(solver, node);

    // tracking variables for the worst approximation
    let mut worst_approximation = f64::NEG_INFINITY;
    let mut index_worst_approximation = 0;

    // scan through the variables and find the worst gain
    for i in 0..solver.qubo.num_x() {
        // if it is a fixed node, then skip it
        if node.fixed_variables.contains_key(&i) {
            continue;
        }

        // find the minimum of the two objective changes
        let min_obj_gain = zero_flip[i].min(one_flip[i]);

        // if it is the highest growing variable, then update the tracking variables
        if min_obj_gain > worst_approximation {
            worst_approximation = min_obj_gain;
            index_worst_approximation = i;
        }
    }

    index_worst_approximation
}

/// Branches on the variable that has an estimated best result,keeping the lower bound as low as possible
pub fn best_approximation(solver: &BBSolver, node: &QuboBBNode) -> usize {
    let (zero_flip, one_flip) = compute_strong_branch(solver, node);

    // tracking variables for the worst approximation
    let mut worst_approximation = f64::INFINITY;
    let mut index_best_approximation = 0;

    // scan through the variables and find the worst gain
    for i in 0..solver.qubo.num_x() {
        // if it is a fixed node, then skip it
        if node.fixed_variables.contains_key(&i) {
            continue;
        }

        // find the minimum of the two objective changes
        let max_obj_gain = zero_flip[i].max(one_flip[i]);

        // if it is the highest growing variable, then update the tracking variables
        if max_obj_gain <= worst_approximation {
            worst_approximation = max_obj_gain;
            index_best_approximation = i;
        }
    }

    index_best_approximation
}

pub fn compute_strong_branch(solver: &BBSolver, node: &QuboBBNode) -> (Array1<f64>, Array1<f64>) {
    let mut base_solution = Array1::zeros(solver.qubo.num_x());
    let mut delta_zero = Array1::zeros(solver.qubo.num_x());
    let mut delta_one = Array1::zeros(solver.qubo.num_x());

    for i in 0..solver.qubo.num_x() {
        // fill in the current vector
        if node.fixed_variables.contains_key(&i) {
            base_solution[i] = *node.fixed_variables.get(&i).unwrap();
        } else {
            base_solution[i] = node.solution[i];
        }

        // compute the delta values for the zero and one flips
        delta_zero[i] = -base_solution[i];
        delta_one[i] = 1.0 - base_solution[i];
    }

    // build the intermediate vectors
    let q_jj = solver.qubo.q.diag().to_dense();
    let q_x = &solver.qubo.q * &base_solution;
    let x_q = &solver.qubo.q.transpose_view() * &base_solution;

    // build the result vectors
    let mut zero_result = Array1::zeros(solver.qubo.num_x());
    let mut one_result = Array1::zeros(solver.qubo.num_x());

    // compute the deltas in the objective compared to the current solution
    for i in 0..solver.qubo.num_x() {
        zero_result[i] = 0.5
            * delta_zero[i]
            * (delta_zero[i] * q_jj[i] + x_q[i] + q_x[i] + 2.0 * solver.qubo.c[i]);
        one_result[i] = 0.5
            * delta_one[i]
            * (delta_one[i] * q_jj[i] + x_q[i] + q_x[i] + 2.0 * solver.qubo.c[i]);
    }

    (zero_result, one_result)
}

/// Wrapper to help convert the QUBO to the format required by Clarabel.rs
pub struct ClarabelWrapper {
    pub q: CscMatrix,
    pub c: Array1<f64>,
}

impl ClarabelWrapper {
    pub fn new(qubo: &Qubo) -> ClarabelWrapper {
        let q_new = ClarabelWrapper::make_cb_form(&(qubo.q));
        ClarabelWrapper {
            q: q_new,
            c: qubo.c.clone(),
        }
    }

    pub fn make_cb_form(p0: &CsMat<f64>) -> CscMatrix {
        let (t, y, u) = p0.to_csc().into_raw_storage();
        CscMatrix::new(p0.rows(), p0.cols(), t, y, u)
    }
}
