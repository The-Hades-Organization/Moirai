use std::time::Instant;

use hades_core::Env;
use moirai_core::limits::limits_now;

fn main() {
    let env = Env::from_process();
    let cwd = std::env::current_dir().unwrap();
    let t = Instant::now();
    let r = limits_now(&env, &cwd);
    println!(
        "limits_now: {:?} ok={} amostras={}",
        t.elapsed(),
        r.is_ok(),
        r.map_or(0, |x| x.amostras)
    );
}
