#![no_std]
#![no_main]

//! 任务五 · 事件驱动验收（EVENT_FILE_MODIFIED）：
//!
//! 演示"文件被修改时唤醒关注的 Agent"这条事件驱动路径,把任务四(文件属性)
//! 与任务五(Agent Loop 事件驱动)打通。
//!
//! 流程:
//! 1. 父进程升级为 Agent,`agent_watch(EVENT_FILE_MODIFIED)` 订阅文件变更事件
//! 2. fork 一个子进程(普通进程,agent_ext 为 None)
//! 3. 子进程稍作延迟后调用 `file_attr_set_tag` 修改文件属性 → 内核广播文件事件
//! 4. 父进程 `agent_wait` 被该事件唤醒,返回原因位含 EVENT_FILE_MODIFIED
//! 5. 父进程再用删除属性触发第二次事件,验证可重复唤醒

#[macro_use]
extern crate user_lib;

use user_lib::{
    EVENT_FILE_MODIFIED, agent_create, agent_watch, agent_wait, exit, file_attr_del_tag,
    file_attr_set_tag, fork, sleep, wait,
};

const FILE: &str = "event_doc";
const TAG: &str = "touched";

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("[demo] ===== Task 5b: file-modified event drives Agent wakeup =====");
    println!("[demo] goal: connect Task 4 (file attrs) with Task 5 (events): modifying a");
    println!("[demo]       file's attributes fires EVENT_FILE_MODIFIED and wakes watchers.");

    if agent_create() != 0 {
        println!("[demo] FAIL: agent_create");
        return 1;
    }
    println!("[demo] SETUP: this (parent) Agent subscribes to EVENT_FILE_MODIFIED...");
    agent_watch(EVENT_FILE_MODIFIED);
    println!("[demo]   -> now watching file-modified events (bit 2)");

    println!("[demo] STEP 1/2: fork a plain child that will modify a file attribute;");
    println!("[demo]          meanwhile parent blocks in agent_wait (TRUE sleep)...");
    let pid = fork();
    if pid == 0 {
        // 子进程:普通进程,延迟后修改文件属性触发事件
        sleep(150);
        let _ = file_attr_set_tag(FILE, TAG);
        println!("[child]   set tag '{}' on '{}' -> kernel broadcasts file event", TAG, FILE);
        exit(0);
    }

    // 父进程:阻塞等待文件事件(真休眠,最多 2s)
    let cause = agent_wait(2000);
    println!(
        "[demo]   -> parent woke up, cause mask={:#x} (EVENT_FILE_MODIFIED={:#x})",
        cause, EVENT_FILE_MODIFIED
    );
    if cause & EVENT_FILE_MODIFIED == 0 {
        println!("[demo]   -> FAIL: expected EVENT_FILE_MODIFIED from child's set");
        return 2;
    }
    println!("[demo]   -> OK: woken by a file SET performed in another process");

    // 回收子进程
    let mut ec = 0;
    let _ = wait(&mut ec);

    // 第二次:本进程删除属性也应触发文件事件,下一次 wait 立即返回
    println!("[demo] STEP 2/2: delete that attribute ourselves; deletion must also fire");
    println!("[demo]          the event, so the next agent_wait returns immediately...");
    let rc = file_attr_del_tag(FILE, TAG);
    println!("[demo]   deleted tag -> rc={} (1=removed)", rc);
    let cause2 = agent_wait(2000);
    println!("[demo]   -> second wait returned, cause mask={:#x}", cause2);
    if cause2 & EVENT_FILE_MODIFIED == 0 {
        println!("[demo]   -> FAIL: expected EVENT_FILE_MODIFIED on delete");
        return 3;
    }
    println!("[demo]   -> OK: woken by a file DELETE");

    println!("[demo] result: both set and delete trigger file events that wake the Agent");
    println!("[demo] PASS task-5b");
    0
}
