#![no_std]
#![no_main]

//! `agent_runner` —— 一键端到端测试入口。
//!
//! 按 demo 顺序 fork+exec 任务 1~6 的全部验收程序，每个 demo 跑完
//! 打印一行总结。最后输出"AGENT-OS ALL DEMOS PASS"。
//!
//! 在 QEMU 里只需输：
//!
//! ```
//! >> agent_runner
//! ```
//!
//! 即可串行跑完所有 demo。

extern crate alloc;
#[macro_use]
extern crate user_lib;

use user_lib::{exec, fork, waitpid};

const DEMOS: &[&str] = &[
    "agent_demo_create\0",
    "agent_demo_coexist\0",
    "agent_demo_tool\0",
    "agent_demo_path\0",
    "agent_demo_file\0",
    "agent_demo_loop\0",
    "agent_demo_npc\0",
];

fn run_one(name_with_null: &str) -> i32 {
    let pid = fork();
    if pid == 0 {
        if exec(name_with_null) == -1 {
            println!("[runner] exec failed: {}", name_with_null.trim_end_matches('\0'));
            return -1;
        }
        unreachable!();
    }
    let mut ec: i32 = 0;
    let _ = waitpid(pid as usize, &mut ec);
    ec
}

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("=============================================");
    println!("  Agent-OS end-to-end demo runner");
    println!("=============================================");

    let mut pass = 0;
    let mut fail = 0;

    for (i, demo) in DEMOS.iter().enumerate() {
        let name = demo.trim_end_matches('\0');
        println!("\n>>> [{}/{}] running {} ...\n", i + 1, DEMOS.len(), name);
        let ec = run_one(demo);
        if ec == 0 {
            println!("\n<<< [{}/{}] {} PASS\n", i + 1, DEMOS.len(), name);
            pass += 1;
        } else {
            println!(
                "\n<<< [{}/{}] {} FAIL (exit={})\n",
                i + 1,
                DEMOS.len(),
                name,
                ec
            );
            fail += 1;
        }
    }

    println!("=============================================");
    println!(
        "  SUMMARY: {} PASS, {} FAIL (out of {})",
        pass,
        fail,
        DEMOS.len()
    );
    if fail == 0 {
        println!("  AGENT-OS ALL DEMOS PASS");
    } else {
        println!("  some demos failed; see logs above");
    }
    println!("=============================================");
    0
}
