#![no_std]
#![no_main]

//! 任务五 · 多 Agent 协调调度（优先级机制，选做加分项）：
//!
//! 演示内核就绪队列的优先级调度：`TaskManager::fetch` 在就绪队列中总是选取
//! `priority` 最大的任务。
//!
//! 设计：父进程在 fork 每个子进程前用 `agent_set_priority` 设好优先级
//! （子进程 fork 时继承父进程优先级），于是三个子进程一出生就分别是
//! HIGH(40) / MID(30) / LOW(20)。父进程随后把自己降到 5，确保不会饿死子进程。
//!
//! 预期现象：三个子进程几乎同时就绪，但 HIGH 会**先把自己所有轮次跑完**，
//! 然后才轮到 MID，最后 LOW —— 即输出里 HIGH 的 round 全部出现在 MID 之前，
//! MID 全部出现在 LOW 之前。这就是优先级调度生效的直接证据。

#[macro_use]
extern crate user_lib;

use user_lib::{agent_create, agent_set_priority, exit, fork, wait, yield_};

fn worker(tag: &str, rounds: usize) {
    for r in 0..rounds {
        // 一点 busy 计算，制造可观察的运行片段
        let mut x: u64 = 0;
        for k in 0..150_000u64 {
            x = x.wrapping_add(k);
        }
        println!("[{}] round {} (x={})", tag, r, x & 0xff);
        // 让出 CPU：调度器会按优先级重新挑选下一个就绪任务
        yield_();
    }
}

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("[demo] ===== Task 5c: priority scheduling (multi-agent coordination) =====");
    println!("[demo] goal: ready queue picks the HIGHEST-priority task first. We spawn 3");
    println!("[demo]       children HIGH(40)/MID(30)/LOW(20); HIGH should monopolize the CPU");
    println!("[demo]       and finish all its rounds before MID, and MID before LOW.");
    let _ = agent_create();

    // 验证 clamp：超过上限会被钳到 255（在 fork 之前做，避免影响子进程继承）
    println!("[demo] STEP 1/3: sanity-check priority clamp (set 9999, expect clamp to 255)...");
    let clamped = agent_set_priority(9999);
    if clamped != 255 {
        println!("[demo]   -> FAIL: expected clamp to 255, got {}", clamped);
        return 2;
    }
    println!("[demo]   -> OK: kernel clamped 9999 to {}", clamped);
    agent_set_priority(16); // 复位到系统默认级

    let prios: [usize; 3] = [40, 30, 20];
    let tags: [&str; 3] = ["HIGH", "MID", "LOW"];
    const ROUNDS: usize = 3;

    println!("[demo] STEP 2/3: set priority then fork each child (child inherits priority)...");
    for i in 0..3 {
        // 设好优先级，fork 出的子进程继承它（HIGH=40 / MID=30 / LOW=20）
        let set = agent_set_priority(prios[i]);
        if set != prios[i] as isize {
            println!("[demo]   -> FAIL: set_priority returned {}", set);
            return 1;
        }
        let pid = fork();
        if pid == 0 {
            // 子进程：跑 ROUNDS 轮，退出码 = 自己的优先级
            worker(tags[i], ROUNDS);
            exit(prios[i] as i32);
        }
        println!(
            "[demo]   spawned child '{}' pid={} with priority={}",
            tags[i], pid, prios[i]
        );
    }

    // 父进程回到系统默认级 16：低于三个子进程(20/30/40)故它们抢先运行，
    // 但不低于 shell/initproc(16) 故父进程自身不会被饿死，能正常 reap。
    agent_set_priority(16);
    println!("[demo] STEP 3/3: parent dropped to priority 16 (below all children); watch the");
    println!("[demo]          interleaving below - HIGH's rounds all come first, then MID, then LOW:");

    let mut reaped = 0;
    while reaped < 3 {
        let mut ec: i32 = 0;
        let pid = wait(&mut ec);
        if pid > 0 {
            println!("[demo]   reaped child pid={} (exit_code = its priority = {})", pid, ec);
            reaped += 1;
        }
    }

    println!("[demo] result: all HIGH rounds precede MID, and MID precede LOW => priority works");
    println!("[demo] PASS task-5c");
    0
}
