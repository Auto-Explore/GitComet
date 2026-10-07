#!/usr/bin/env python3
"""Compare published native Mermaid rendering with GitComet's production path."""
import argparse
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = r'''
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{alloc::System, time::Instant};
#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
fn main() {
    println!("case,cold_us,cold_alloc_ops,cold_alloc_bytes,warm_mean_us,warm_alloc_ops,warm_alloc_bytes,svg_bytes");
    for (name,count,dense) in [("small",4,false),("typical",50,false),("dense",100,true),("large",500,false)] {
        let mut source=String::from("flowchart LR\n");
        for i in 0..count {
            source.push_str(&format!("N{i}[Repeated label]-->N{}\n",i+1));
            if dense && i>2 { source.push_str(&format!("N{}-.->N{i}\n",i-3)); }
        }
        let cold=Region::new(GLOBAL); let start=Instant::now();
        let first=svg(&source); let cold_us=start.elapsed().as_micros(); let cold_stats=cold.change();
        let warm=Region::new(GLOBAL); let start=Instant::now();
        for _ in 0..20 { std::hint::black_box(svg(&source)); }
        let warm_us=start.elapsed().as_micros()/20; let warm_stats=warm.change();
        println!("{name},{cold_us},{},{},{warm_us},{},{},{}",cold_stats.allocations,cold_stats.bytes_allocated,
            warm_stats.allocations/20,warm_stats.bytes_allocated/20,first.len());
    }
}
'''
BEFORE = '''
fn svg(source: &str) -> std::sync::Arc<[u8]> {
    let svg=mermaid_rs_renderer::render_strict(source,Default::default()).unwrap();
    let doc=roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(doc.root_element().tag_name().name(),"svg");
    svg.into_bytes().into()
}
'''
AFTER = '''
fn svg(source: &str) -> std::sync::Arc<[u8]> {
    gitcomet_diagrams::render(gitcomet_diagrams::DiagramKind::Mermaid,source)
        .unwrap().pages.remove(0).svg
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / "target/mermaid-profile")
    args = parser.parse_args()
    output = args.output.resolve()
    for name, dependency, render in [
        ("before", 'mermaid-rs-renderer = { version = "=0.3.1", default-features = false }\nroxmltree = "0.20"', BEFORE),
        ("after", 'gitcomet-diagrams = { path = ' + json.dumps(str(ROOT / "crates/gitcomet-diagrams")) + ' }', AFTER),
    ]:
        project = output / name
        (project / "src").mkdir(parents=True, exist_ok=True)
        (project / "Cargo.toml").write_text(
            '[package]\nname = "mermaid-' + name + '"\nversion = "0.0.0"\nedition = "2024"\n'
            '[workspace]\n[dependencies]\nstats_alloc = "0.1.10"\n' + dependency + '\n'
            '[profile.release]\nlto = "fat"\ncodegen-units = 1\n'
        )
        (project / "src/main.rs").write_text(SOURCE + render)
        subprocess.run(["cargo", "build", "--release", "--offline", "--manifest-path", str(project / "Cargo.toml")], check=True)
    # Build both first: neither measurement overlaps compilation.
    for name in ["before", "after"]:
        project = output / name
        result = subprocess.run([str(project / "target/release" / ("mermaid-" + name))], check=True, text=True, capture_output=True)
        (output / (name + ".csv")).write_text(result.stdout)
        print(name + "\n" + result.stdout, end="")


if __name__ == "__main__":
    main()
