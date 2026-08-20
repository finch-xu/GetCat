//! 性能回归（手动运行，默认 #[ignore]）：100 MB 合成 JSON 的美化 + 建索引 < 2 s，自有内存 < 3 × 输入。
//!
//!   cargo test -p getcat-core --release --test perf_large_body -- --ignored --nocapture
//!
//! 断言的"自有内存"= 输入缓冲 + 美化输出缓冲（按 capacity 计）+ 行索引，是本进程为这份数据实际持有的堆内存；
//! 它不包含分配器碎片与运行时基线。要看真实峰值 RSS，用：
//!
//!   /usr/bin/time -l cargo test -p getcat-core --release --test perf_large_body -- --ignored 2>&1 | grep "maximum resident"
//!
//! 预期 "maximum resident set size" 约为 3 × 100 MiB + 编译产物 / 运行时基线（< 400 MB）。
//! 注意：美化输出的膨胀率取决于 JSON 形态——此处用字符串占比高的"记录型"JSON（≈1.45×）；
//! 以小数组、小对象为主的 token 密集型 JSON 可达 2× 以上，届时 3× 预算由 spill 阈值（64 MiB）兜住。

use std::time::Instant;

use getcat_core::body::{pretty::pretty_json, text::TextDoc};

fn synthetic_json(target: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(target + 256);
    out.push(b'[');
    let mut i = 0u64;
    while out.len() < target {
        if i > 0 {
            out.push(b',');
        }
        out.extend_from_slice(
            format!(
                r#"{{"id":{i},"name":"user-{i}","email":"user-{i}@example.com","score":{},"active":{},"note":"lorem ipsum dolor sit amet, consectetur adipiscing elit"}}"#,
                i % 100,
                i.is_multiple_of(2)
            )
            .as_bytes(),
        );
        i += 1;
    }
    out.push(b']');
    out
}

#[test]
#[ignore = "performance regression; run manually with --release"]
fn pretty_and_index_100mb_under_two_seconds() {
    let input = synthetic_json(100 * 1024 * 1024);
    let input_len = input.len();

    let started = Instant::now();
    let pretty = pretty_json(&input);
    let doc = TextDoc::from_bytes(pretty);
    let elapsed = started.elapsed();

    let owned = input.capacity() + doc.heap_bytes();
    eprintln!(
        "input {:.1} MiB -> pretty {:.1} MiB, {} lines, {:?}, owned {:.1} MiB ({:.2}x)",
        input_len as f64 / 1048576.0,
        doc.len_bytes() as f64 / 1048576.0,
        doc.line_count(),
        elapsed,
        owned as f64 / 1048576.0,
        owned as f64 / input_len as f64,
    );
    assert!(elapsed.as_secs_f64() < 2.0, "took {elapsed:?}");
    assert!(
        owned < 3 * input_len,
        "owned {owned} bytes exceeds 3 x input ({input_len})"
    );
}
