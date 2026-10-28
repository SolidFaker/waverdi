use std::fs;
use std::path::Path;

use super::super::SourceSet;
use super::lexer::{lex, Tok};
use super::parser::{parse_int, parse_module_text, range_width};
use super::*;
use crate::waveform::Waveform;

const COUNTER: &str = r#"
// A small counter used by the demo.
module counter #(parameter int WIDTH = 8) (
    input  logic clk,
    input  logic rst_n,
    input  logic en,
    output logic [WIDTH-1:0] count,
    output logic carry
);
    logic [WIDTH-1:0] next;

    assign carry = en && (count == {WIDTH{1'b1}});
    assign next = count + 1'b1;

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) count <= '0;
        else if (en) count <= next;
    end

    /* a nested module mentioned in a comment must be ignored:
       module not_real; endmodule */
endmodule
"#;

fn parse_counter() -> ModuleDef {
    parse_module_text(COUNTER, Path::new("counter.sv"))
        .into_iter()
        .next()
        .expect("module")
}

#[test]
fn parses_module_header_and_declarations() {
    let m = parse_counter();
    assert_eq!(m.name, "counter");
    assert_eq!(m.start, 3);
    assert_eq!(m.ports.len(), 5);
    let count = m.signal("count").unwrap();
    assert_eq!(count.direction.as_deref(), Some("output"));
    assert_eq!(count.kind, "logic");
    assert_eq!(count.range.as_deref(), Some("WIDTH-1:0"));
    assert_eq!(
        m.signal("next").unwrap().range.as_deref(),
        Some("WIDTH-1:0")
    );
}

#[test]
fn large_declaration_lists_preserve_first_and_port_merging() {
    let count = 20_000usize;
    let mut text = String::from("module big(input logic first, output logic last);\n");
    for index in 0..count {
        text.push_str(&format!("    logic d{index};\n"));
    }
    text.push_str("    logic [7:0] first;\n    logic d0, d1;\nendmodule\n");

    let module = parse_module_text(&text, Path::new("declarations.sv"))
        .into_iter()
        .next()
        .expect("module");
    assert_eq!(module.ports.len(), 2);
    assert_eq!(module.signals.len(), count + 2);
    let first = module.signal("first").expect("header declaration");
    assert_eq!(first.direction.as_deref(), Some("input"));
    assert_eq!(first.range.as_deref(), None);
    assert_eq!(first.line, 1);
    assert_eq!(module.signal("d0").unwrap().line, 2);
    assert_eq!(
        module.signal("last").unwrap().direction.as_deref(),
        Some("output")
    );
}

#[test]
fn parses_assignments_and_always_blocks() {
    let m = parse_counter();
    assert_eq!(m.assigns.len(), 2);
    assert_eq!(m.assigns[0].lhs, "carry");
    assert!(m.assigns[0].uses.iter().any(|(name, _)| name == "count"));
    assert_eq!(m.always.len(), 1);
    let block = &m.always[0];
    assert!(block.sensitivity.iter().any(|(name, _)| name == "clk"));
    assert!(block.sensitivity.iter().any(|(name, _)| name == "rst_n"));
    assert!(block.drivers.iter().any(|(name, _)| name == "count"));
    // The `count == ...` comparison inside assign must not look like a load.
    assert_eq!(m.instances.len(), 0);
}

#[test]
fn parses_instantiations() {
    let text = r#"
module top;
    logic clk, rst_n, en, carry;
    wire [7:0] count;
    counter #(.WIDTH(8)) u_dut (
        .clk(clk),
        .rst_n(rst_n),
        .en(en),
        .count(count),
        .carry(carry)
    );
    assign en = 1'b1;
endmodule
"#;
    let m = parse_module_text(text, Path::new("top.sv"))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(m.instances.len(), 1);
    assert_eq!(m.instances[0].module, "counter");
    assert_eq!(m.instances[0].name, "u_dut");
    assert_eq!(m.instances[0].line, 5);
    assert_eq!(m.instances[0].ports.len(), 5);
    assert!(m.instances[0]
        .ports
        .iter()
        .any(|(name, line)| name == "clk" && *line == 6));
    assert!(m.instances[0]
        .ports
        .iter()
        .any(|(name, line)| name == "carry" && *line == 10));
}

#[test]
fn part_select_ranges_report_the_width_after_the_colon() {
    let module = parse_module_text("module m; endmodule\n", Path::new("m.sv"))
        .into_iter()
        .next()
        .expect("module");
    assert_eq!(range_width("4+:3", &module), Some(3));
    assert_eq!(range_width("4-:3", &module), Some(3));
    // Plain ranges keep their inclusive width.
    assert_eq!(range_width("7:0", &module), Some(8));
}

#[test]
fn walk_scope_budget_bounds_nested_indexless_loops() {
    // Both loops are described by dump steps without an index, so the
    // search recurses into every value of every level: the missing tail
    // step would explore `4096 * 4096` paths without a budget.
    let text = r#"
module top;
    generate
        for (genvar i = 0; i < 4096; i++) begin : a
            for (genvar j = 0; j < 4096; j++) begin : b
                leaf u_leaf();
            end
        end
    endgenerate
endmodule

module leaf;
    logic y;
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("waverdi_walk_budget_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("gen.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    let steps = |names: &[&str]| {
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    };
    // The unresolvable tail must terminate through the budget, not spin
    // through the loop product.
    let start = std::time::Instant::now();
    assert!(db
        .module_at_scope(&steps(&["top", "a", "b", "missing"]))
        .is_none());
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "bounded search must return quickly"
    );
    // A matching path inside the same loops still resolves.
    assert_eq!(
        db.module_at_scope(&steps(&["top", "a", "b", "u_leaf"]))
            .unwrap()
            .name,
        "leaf"
    );
}

#[test]
fn walk_scope_budget_keeps_shallow_matches() {
    let text = r#"
module top;
    generate
        for (genvar i = 0; i < 512; i++) begin : a
            leaf u_leaf();
        end
    endgenerate
endmodule

module leaf;
    logic y;
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("waverdi_walk_shallow_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("gen.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    let steps = |names: &[&str]| {
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    };
    let found = db
        .scope_info(&steps(&["top", "a", "u_leaf"]))
        .expect("loop instance inside an index-less generate block");
    assert_eq!(found.module.name, "leaf");
    assert_eq!(found.instance_scope, steps(&["top", "a[0]", "u_leaf"]));
}

/// The hierarchy caches memoize `top_modules`, `default_env` and
/// `scope_info`: a repeated query must reuse them instead of walking the
/// design again, and a fresh database starts empty.
#[test]
fn hierarchy_lookups_are_memoized() {
    let dir = std::env::temp_dir().join(format!("waverdi_rtl_memo_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mem.sv");
    fs::write(
        &file,
        "module top;\n\
             logic clk;\n\
             leaf u_leaf(.clk(clk));\n\
             endmodule\n\
             module leaf;\n\
             logic x;\n\
             endmodule\n",
    )
    .unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    assert_eq!(db.cache_sizes(), (false, 0, 0));

    let steps = vec!["top".to_string(), "u_leaf".to_string()];
    let first = db.scope_info(&steps).expect("leaf instance");
    assert_eq!(first.module.name, "leaf");
    let (has_tops, envs, scopes) = db.cache_sizes();
    assert!(has_tops, "top_modules must be memoized");
    assert!(envs > 0 && scopes == 1, "{envs} {scopes}");

    // The second query returns the same match without new cache entries.
    let second = db.scope_info(&steps).expect("memoized leaf instance");
    assert_eq!(second.module.name, first.module.name);
    assert_eq!(second.instance_scope, first.instance_scope);
    assert_eq!(db.cache_sizes(), (has_tops, envs, scopes));

    // `default_env` is memoized per module as well.
    let top = db.module("top").unwrap();
    assert_eq!(db.default_env(top), db.default_env(top));
    assert_eq!(db.cache_sizes().1, envs);
}

#[test]
fn simulation_types_declare_signals() {
    let text = "module m;\n\
                    \x20   int a;\n\
                    \x20   int unsigned b;\n\
                    \x20   int signed c = 0;\n\
                    \x20   integer d;\n\
                    \x20   real e;\n\
                    \x20   time f;\n\
                    \x20   shortint g;\n\
                    \x20   longint h;\n\
                    \x20   byte i;\n\
                    endmodule\n";
    let module = parse_module_text(text, Path::new("m.sv"))
        .into_iter()
        .next()
        .unwrap();
    for name in ["a", "b", "c", "d", "e", "f", "g", "h", "i"] {
        assert!(module.declares(name), "{name} not declared");
    }
}

#[test]
fn inactive_generate_branches_report_their_lines() {
    let text = "\
module top;
    generate
        if (0) begin : off
            assign a = b;
        end else begin : on
            assign c = d;
        end
    endgenerate
endmodule

module pick;
    generate
        if (1) begin : on
            assign e = f;
        end else begin : off
            assign g = h;
        end
    endgenerate
endmodule

module unknown;
    generate
        if (MAYBE) begin : a
            assign i = j;
        end else begin : b
            assign k = l;
        end
    endgenerate
endmodule

module alone;
    generate
        if (0) begin : dead
            assign m = n;
        end
    endgenerate
endmodule
";
    let dir = std::env::temp_dir().join(format!("waverdi_inactive_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("gen.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    // `if (0)`: the first branch is dead, the `else` is instantiated.
    assert_eq!(db.inactive_lines("top"), vec![3..5]);
    // `if (1)`: the `else` branch is dead.
    assert_eq!(db.inactive_lines("pick"), vec![15..18]);
    // An unknown condition cannot rule either branch out.
    assert!(db.inactive_lines("unknown").is_empty());
    // A dead branch without an `else` is the whole construct.
    assert_eq!(db.inactive_lines("alone"), vec![33..36]);
}

#[test]
fn traces_signals_to_declaration_and_drivers() {
    let dir = std::env::temp_dir().join(format!("waverdi_scan_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("counter.sv");
    fs::write(&file, COUNTER).unwrap();
    let set = SourceSet::from_files(vec![file.clone()], "test");
    let db = RtlDb::parse_sources(&set);
    let trace = db.trace("counter", "count").expect("trace");
    assert_eq!(trace.decl.as_ref().unwrap().line, 7);
    assert!(trace.drivers.iter().any(|loc| loc.line == 16));
    assert!(trace.loads.iter().any(|loc| loc.line == 12));
    assert!(trace.loads.iter().any(|loc| loc.line == 13));
    assert!(db.module("counter").is_some());
}

#[test]
fn resolves_struct_members_and_instance_aggregates() {
    let text = r#"
module m(input logic clk);
    typedef struct packed { logic run; logic [3:0] pc; } fetch_t;
    fetch_t clk_fetch;
    fetch_t clk_pc;
    child u_child(.clk(clk));
endmodule

module child(input logic clk);
endmodule

module top(input logic clk);
    if_t vif(.clk(clk));
    assign y = vif.valid;
endmodule

interface if_t(input logic clk);
    logic valid;
endinterface
"#;
    let dir = std::env::temp_dir().join(format!("waverdi_sv_member_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("m.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);

    // Struct variable member: the chain walks the dump scope of the
    // variable (`clk_fetch.run`).
    let (path, signal) = db
        .resolve_reference("m", &["clk_fetch".to_string()], "run")
        .expect("struct member");
    assert_eq!(path, vec!["clk_fetch".to_string()]);
    assert_eq!(signal, "run");
    // Whole struct variable.
    let (path, signal) = db
        .resolve_reference("m", &[], "clk_fetch")
        .expect("struct variable");
    assert!(path.is_empty());
    assert_eq!(signal, "clk_fetch");
    // Interface instance member resolves through the instance scope.
    let (path, signal) = db
        .resolve_reference("top", &["vif".to_string()], "valid")
        .expect("interface member");
    assert_eq!(path, vec!["vif".to_string()]);
    assert_eq!(signal, "valid");
    // A whole instance resolves to its scope aggregate.
    let (path, signal) = db.resolve_reference("top", &[], "vif").expect("instance");
    assert!(path.is_empty());
    assert_eq!(signal, "vif");
}

#[test]
fn nested_generate_fallback_is_bounded() {
    let mut text = String::from("module top;\n    generate\n");
    for (var, label) in [("i", "a"), ("j", "b"), ("k", "c")] {
        text.push_str(&format!(
            "        for (genvar {var} = 0; {var} < 128; {var}++) begin : {label}\n"
        ));
    }
    text.push_str("            leaf u_leaf();\n");
    for _ in 0..3 {
        text.push_str("        end\n");
    }
    text.push_str("    endgenerate\nendmodule\n\nmodule leaf;\n    logic y;\nendmodule\n");
    let dir = std::env::temp_dir().join(format!("waverdi_gen_bound_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("gen.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    // An unresolved identifier inside nested loops must terminate quickly.
    assert!(db
        .resolve_reference("top", &["missing".to_string()], "x")
        .is_none());
    // A real instance inside the loops still resolves to its first path.
    let (path, signal) = db
        .resolve_reference("top", &["u_leaf".to_string()], "y")
        .expect("nested instance");
    assert_eq!(
        path,
        vec![
            "a[0]".to_string(),
            "b[0]".to_string(),
            "c[0]".to_string(),
            "u_leaf".to_string()
        ]
    );
    assert_eq!(signal, "y");
}

#[test]
fn real_world_header_with_params_and_interface_ports() {
    let text = r#"`default_nettype none

module prt_dp_pm_top
#
(
    parameter                           P_VENDOR        = "none",    // Vendor - "AMD", "ALTERA" or "LSC" 
    parameter                           P_BEAT          = 'd125,     // Beat value
    parameter                           P_HW_VER_MAJOR  = 1,         // Hardware version major
    parameter                           P_HW_VER_MINOR  = 0,         // Hardware version minor
    parameter                           P_CFG           = "tx",      // Configuration TX / RX
    parameter                           P_SIM           = 0,
    parameter                           P_ROM_INIT_FILE = "none",
    parameter                           P_RAM_INIT_FILE = "none",
    parameter                           P_PIO_IN_WIDTH  = 8,
    parameter                           P_PIO_OUT_WIDTH = 8,
    parameter                           P_SPL = 2,                   // Symbols per lane
    parameter                           P_MST = 0                    // MST
)
(
    // Reset and clock
    input wire                          RST_IN,
    input wire                          CLK_IN,

    // Interrupt
    input wire [1:0]                    IRQ_IN,

    // PIO
    input wire [P_PIO_IN_WIDTH-1:0]     PIO_IN,
    output wire [P_PIO_OUT_WIDTH-1:0]   PIO_OUT,

    // Host
    prt_dp_lb_if.lb_in                  HOST_IF,
    output wire                         HOST_IRQ_OUT, 

    // HPD
    input wire                          HPD_IN,
    output wire                         HPD_OUT,

    // AUX
    output wire                         AUX_EN_OUT,
    output wire                         AUX_TX_OUT,
    input wire                          AUX_RX_IN,

    // Message 
    prt_dp_msg_if.src                   MSG_SRC_IF,
    prt_dp_msg_if.snk                   MSG_SNK_IF
);
endmodule
"#;
    let modules = parse_module_text(text, Path::new("prt_dp_pm_top.sv"));
    assert_eq!(modules.len(), 1);
    let m = &modules[0];
    assert_eq!(m.params.len(), 12);
    let ports: Vec<&str> = m.ports.iter().map(|decl| decl.name.as_str()).collect();
    assert_eq!(
        ports,
        vec![
            "RST_IN",
            "CLK_IN",
            "IRQ_IN",
            "PIO_IN",
            "PIO_OUT",
            "HOST_IF",
            "HOST_IRQ_OUT",
            "HPD_IN",
            "HPD_OUT",
            "AUX_EN_OUT",
            "AUX_TX_OUT",
            "AUX_RX_IN",
            "MSG_SRC_IF",
            "MSG_SNK_IF"
        ]
    );
}

#[test]
fn header_ports_with_interface_types() {
    let text = r#"
module prt_dp_pm_top
#
(
    parameter P_VENDOR = "none",
    parameter P_CFG = "tx",
    parameter P_PIO_IN_WIDTH = 8,
    parameter P_MST = 0
)
(
    input wire RST_IN,
    input wire [1:0] IRQ_IN,
    input wire [P_PIO_IN_WIDTH-1:0] PIO_IN,
    prt_dp_lb_if.lb_in HOST_IF,
    output wire HOST_IRQ_OUT,
    prt_dp_msg_if.src MSG_SRC_IF
);
endmodule
"#;
    let modules = parse_module_text(text, Path::new("t.sv"));
    let m = &modules[0];
    let ports: Vec<&str> = m.ports.iter().map(|decl| decl.name.as_str()).collect();
    assert_eq!(
        ports,
        vec![
            "RST_IN",
            "IRQ_IN",
            "PIO_IN",
            "HOST_IF",
            "HOST_IRQ_OUT",
            "MSG_SRC_IF"
        ]
    );
}

#[test]
fn scope_info_reports_generate_block_lines() {
    let text = r#"
module top;
    generate
        for (genvar i = 0; i < 2; i++) begin : gen_a
            child u_child();
        end
    endgenerate
    if (1) begin : gen_b
        child u_b();
    end
endmodule

module child;
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("waverdi_gen_line_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("top.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    // `begin : gen_a` is on line 4 of the file.
    let found = db
        .scope_info(&["top".to_string(), "gen_a[1]".to_string()])
        .expect("generate block");
    assert_eq!(found.line, Some(4));
    // Nested if-block: deepest block line wins.
    let found = db
        .scope_info(&["top".to_string(), "gen_b".to_string()])
        .expect("if block");
    assert_eq!(found.line, Some(8));
    // Paths that end at an instance report the instance boundary instead.
    let found = db
        .scope_info(&[
            "top".to_string(),
            "gen_a[0]".to_string(),
            "u_child".to_string(),
        ])
        .expect("instance inside generate");
    assert_eq!(found.instance_scope.len(), 3);
}

#[test]
fn missing_generate_scopes_are_merged_into_the_tree() {
    let text = r#"
module top #(parameter int N = 3) (input logic clk);
    generate
        for (genvar i = 0; i < N; i++) begin : gen_x
            logic flag;
        end
    endgenerate
    generate
        if (N > 1) begin : gen_y
            logic y;
        end
    endgenerate
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("waverdi_gen_merge_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("top.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    let mut wf = Waveform {
        ts: crate::waveform::TimeScale::default(),
        start: 0,
        end: 0,
        signals: vec![],
        tree: crate::waveform::ScopeTree::new(),
        radix: std::collections::HashMap::new(),
        value_times_cache: Vec::new(),
    };
    let top = wf
        .tree
        .add_scope(wf.tree.root, "top".to_string(), "top".to_string());
    // One iteration already exists in the dump; it must be reused.
    wf.tree
        .add_scope(top, "gen_x[1]".to_string(), String::new());
    db.merge_generate_scopes(&mut wf);
    let names: Vec<&str> = wf.tree.nodes[top]
        .children
        .iter()
        .map(|&child| wf.tree.nodes[child].name.as_str())
        .collect();
    assert_eq!(names, vec!["gen_x[1]", "gen_x[0]", "gen_x[2]", "gen_y"]);
    assert_eq!(names.iter().filter(|name| **name == "gen_x[1]").count(), 1);
}

#[test]
fn large_generated_case_tables_parse_without_quadratic_blowup() {
    // Generated video overlay tables are single always blocks with
    // hundreds of thousands of case items; use/trace filtering must not
    // be quadratic in the number of items.
    let items = 20_000;
    let mut text = String::from("module big(input logic clk, output logic data);\n");
    text.push_str("    always_ff @(posedge clk) begin\n        case (clk)\n");
    for i in 0..items {
        text.push_str(&format!("            32'd{i} : data <= 1'b0;\n"));
    }
    text.push_str("        endcase\n    end\n");
    for i in 0..items {
        text.push_str(&format!("    assign use_{i} = data;\n"));
    }
    text.push_str("endmodule\n");

    let dir = std::env::temp_dir().join(format!("waverdi_big_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("big.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);

    let m = db.module("big").expect("module");
    assert_eq!(m.always.len(), 1);
    assert_eq!(m.always[0].drivers.len(), items);
    assert_eq!(m.assigns.len(), items);
    let trace = db.trace("big", "data").expect("trace");
    assert_eq!(trace.drivers.len(), items);
    assert_eq!(trace.loads.len(), items);
}

#[test]
fn elaborates_the_design_hierarchy_and_resolves_references() {
    let text = r#"
module counter(input logic clk, output logic [3:0] count);
    logic [3:0] next;
    assign next = count;
endmodule

module tb;
    logic clk;
    counter u_dut(.clk(clk));
    assign x = u_dut.count;
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("waverdi_elab_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("tb.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    assert_eq!(db.top_modules(), vec!["tb".to_string()]);
    let steps = |names: &[&str]| {
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(db.module_at_scope(&steps(&["tb"])).unwrap().name, "tb");
    assert_eq!(
        db.module_at_scope(&steps(&["tb", "u_dut"])).unwrap().name,
        "counter"
    );
    assert!(db.module_at_scope(&steps(&["nobody"])).is_none());
    // Plain references only resolve where the AST declares them.
    assert!(db.resolve_reference("tb", &[], "count").is_none());
    assert!(db.resolve_reference("counter", &[], "count").is_some());
    // Qualified references walk the instance hierarchy.
    let (path, signal) = db
        .resolve_reference("tb", &steps(&["u_dut"]), "count")
        .expect("qualified");
    assert_eq!(path, vec!["u_dut".to_string()]);
    assert_eq!(signal, "count");
    // An instance name resolves to the aggregate of its dump scope.
    let (path, signal) = db
        .resolve_reference("tb", &[], "u_dut")
        .expect("instance aggregate");
    assert!(path.is_empty());
    assert_eq!(signal, "u_dut");
    // Elaborated placements cover the whole design, with top first.
    let placements = db.placements();
    assert!(placements.contains(&(steps(&["tb"]), "tb".to_string())));
    assert!(placements.contains(&(steps(&["tb", "u_dut"]), "counter".to_string())));
}

#[test]
fn elaborates_generate_loops_and_branches() {
    let text = r#"
module leaf(input logic clk, output logic [3:0] q);
    logic [3:0] next;
    assign next = q;
endmodule

module mid #(parameter int N = 3, parameter bit FAST = 1'b1) (
    input logic clk
);
    genvar i;
    generate
        for (i = 0; i < N; i++) begin : gen_leaves
            if ((i % 2) == 0) begin : gen_even
                leaf u_even(.clk(clk), .q());
            end else begin : gen_odd
                leaf u_odd(.clk(clk), .q());
            end
        end
    endgenerate
    generate
        if (FAST) begin : gen_fast
            leaf u_fast(.clk(clk), .q());
        end else begin : gen_slow
            leaf u_slow(.clk(clk), .q());
        end
    endgenerate
endmodule

module top;
    logic clk;
    mid #(.N(4), .FAST(1'b0)) u_mid(.clk(clk));
    raw u_raw_top(.clk(clk));
endmodule

module raw(input logic clk);
    genvar r;
    generate
        for (r = 0; r < 2; r++) begin
            leaf u_raw(.clk(clk), .q());
        end
    endgenerate
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("waverdi_gen_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("gen.sv");
    fs::write(&file, text).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    assert_eq!(db.top_modules(), vec!["top".to_string()]);

    let steps = |names: &[&str]| {
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    };
    // Parameters: default N=3, overridden to 4 through the instance.
    assert_eq!(db.module("mid").unwrap().params["N"], Some(3));
    // The loop is unrolled with the override; the else-branch is taken.
    let placements = db.placements();
    assert!(placements.contains(&(
        steps(&["top", "u_mid", "gen_leaves[0]", "gen_even", "u_even"]),
        "leaf".to_string()
    )));
    assert!(placements.contains(&(
        steps(&["top", "u_mid", "gen_leaves[3]", "gen_odd", "u_odd"]),
        "leaf".to_string()
    )));
    assert!(!placements
        .iter()
        .any(|(path, _)| path.contains(&"gen_leaves[4]".to_string())));
    assert!(placements.contains(&(
        steps(&["top", "u_mid", "gen_slow", "u_slow"]),
        "leaf".to_string()
    )));
    assert!(!placements
        .iter()
        .any(|(path, _)| path.contains(&"gen_fast".to_string())));
    // Unnamed generate loops use the tool-assigned genblk numbering.
    assert!(placements.contains(&(
        steps(&["top", "u_raw_top", "genblk1[0]", "u_raw"]),
        "leaf".to_string()
    )));
    assert!(placements.contains(&(
        steps(&["top", "u_raw_top", "genblk1[1]", "u_raw"]),
        "leaf".to_string()
    )));
    assert_eq!(placements.len(), 10);

    // Dump scope paths walk through loops and branches.
    assert_eq!(
        db.module_at_scope(&steps(&[
            "top",
            "u_mid",
            "gen_leaves[2]",
            "gen_even",
            "u_even"
        ]))
        .unwrap()
        .name,
        "leaf"
    );
    assert_eq!(
        db.module_at_scope(&steps(&["top", "u_mid", "gen_slow", "u_slow"]))
            .unwrap()
            .name,
        "leaf"
    );
    assert!(db
        .module_at_scope(&steps(&["top", "u_mid", "gen_fast", "u_fast"]))
        .is_none());
    assert_eq!(
        db.module_at_scope(&steps(&["top", "u_raw_top", "genblk1[1]", "u_raw"]))
            .unwrap()
            .name,
        "leaf"
    );
    // Unknown helper scopes are skipped.
    assert_eq!(
        db.module_at_scope(&steps(&[
            "top",
            "u_mid",
            "unnamed$$_0",
            "gen_leaves[1]",
            "gen_odd"
        ]))
        .unwrap()
        .name,
        "mid"
    );

    // References can walk into generated instances.
    let (path, signal) = db
        .resolve_reference(
            "top",
            &steps(&["u_mid", "gen_leaves[1]", "gen_odd", "u_odd"]),
            "q",
        )
        .expect("generated reference");
    assert_eq!(path, steps(&["u_mid", "gen_leaves[1]", "gen_odd", "u_odd"]));
    assert_eq!(signal, "q");
    // Plain instance names inside a loop resolve to the first iteration.
    let (path, _) = db
        .resolve_reference("mid", &steps(&["u_even"]), "q")
        .expect("loop instance");
    assert_eq!(path, steps(&["gen_leaves[0]", "gen_even", "u_even"]));
    // Unnamed loops use the genblk naming in returned paths too.
    let (path, _) = db
        .resolve_reference("raw", &steps(&["u_raw"]), "q")
        .expect("unnamed loop instance");
    assert_eq!(path, steps(&["genblk1[0]", "u_raw"]));

    // scope_info reports the bindings and instance scope behind a
    // generate block scope.
    let found = db
        .scope_info(&steps(&["top", "u_mid", "gen_leaves[1]"]))
        .expect("genblk scope");
    assert_eq!(found.module.name, "mid");
    assert_eq!(found.instance_scope, steps(&["top", "u_mid"]));
    assert_eq!(found.env.get("i"), Some(&1));
    // A genvar bound by the selected scope pins plain instance references.
    let (path, _) = db
        .resolve_reference_with("mid", &steps(&["u_odd"]), "q", &found.env)
        .expect("pinned iteration");
    assert_eq!(path, steps(&["gen_leaves[1]", "gen_odd", "u_odd"]));
    // Index selectors are constant expressions over those bindings.
    assert_eq!(eval(&parse_expr_text("i").unwrap(), &found.env), Some(1));
}

#[test]
fn always_blocks_with_begin_end_do_not_end_the_module() {
    let text = r#"
module pipe_reg(input logic clk, input logic rst_n, output logic q);
    logic r;
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            r <= 1'b0;
        end else begin
            r <= ~r;
        end
    end
    assign q = r;
endmodule

module fifo(input logic clk, output logic [3:0] count);
    logic [3:0] c;
    always_ff @(posedge clk) begin
        c <= c + 1'b1;
    end
    assign count = c;
endmodule
"#;
    let modules = parse_module_text(text, Path::new("common.sv"));
    assert_eq!(modules.len(), 2);
    let pipe = &modules[0];
    assert_eq!(pipe.name, "pipe_reg");
    assert_eq!(pipe.end, 12); // the module ends at its `endmodule`
    assert!(pipe.signal("r").is_some());
    assert!(pipe.assigns.iter().any(|assign| assign.lhs == "q"));
    assert!(pipe.always[0]
        .drivers
        .iter()
        .any(|(name, line)| name == "r" && *line == 6));
    let fifo = &modules[1];
    assert_eq!(fifo.name, "fifo");
    assert_eq!(fifo.start, 14);
    assert_eq!(fifo.end, 20);
    assert!(fifo.signal("c").is_some());
}

#[test]
fn indexed_assignments_are_driven_by_their_base_signal() {
    let text = r#"
module m(input logic clk);
    logic [7:0] mem [2][2];
    always_ff @(posedge clk) begin
        mem[a][b] <= 8'h01;
    end
endmodule
"#;
    let modules = parse_module_text(text, Path::new("m.sv"));
    let block = &modules[0].always[0];
    assert!(block
        .drivers
        .iter()
        .any(|(name, line)| name == "mem" && *line == 5));
    assert!(!block.drivers.iter().any(|(name, _)| name == "b"));
}

#[test]
fn ignores_strings_and_directives() {
    let text = r#"
`timescale 1ns/1ps
module small;
    initial $display("module fake; endmodule");
endmodule
"#;
    let modules = parse_module_text(text, Path::new("small.sv"));
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].name, "small");
}

const SV_TYPES: &str = r#"
package cfg_pkg;
    parameter int WIDTH = 16;
    localparam int DEPTH = 4;
    typedef enum logic [1:0] {S_IDLE, S_RUN} state_e;
    typedef struct packed { logic [WIDTH-1:0] data; logic last; } beat_t;
endpackage

interface lb_if (input logic clk);
    logic valid;
    modport master (output valid, input clk);
endinterface

module top #(parameter int N = cfg_pkg::WIDTH) (input logic clk);
    import cfg_pkg::*;
    typedef logic [7:0] byte_t;
    state_e state;
    beat_t beat;
    byte_t a, b;
    lb_if u_if (.clk(clk));
    lb_if.master bus;
    generate
        for (genvar i = 0; i < DEPTH; i++) begin : g
            sub u_sub (.clk(clk));
        end
    endgenerate
endmodule

module sub(input logic clk);
    logic flag;
endmodule
"#;

#[test]
fn parses_interfaces_packages_and_typedefs() {
    let modules = parse_module_text(SV_TYPES, Path::new("sv.sv"));
    assert_eq!(modules.len(), 4);
    let package = &modules[0];
    assert_eq!(package.name, "cfg_pkg");
    assert_eq!(package.kind, DefKind::Package);
    assert_eq!(package.params.get("WIDTH"), Some(&Some(16)));
    assert_eq!(package.params.get("DEPTH"), Some(&Some(4)));
    assert_eq!(package.typedefs.get("state_e").unwrap().width, Some(2));

    let interface = &modules[1];
    assert_eq!(interface.kind, DefKind::Interface);
    assert!(interface.signal("valid").is_some());

    let top = &modules[2];
    assert_eq!(top.kind, DefKind::Module);
    assert!(top.imports.contains(&"cfg_pkg".to_string()));
    // User-typed declarations become signals, not instances.
    assert!(top.signal("state").is_some());
    assert_eq!(top.signal("state").unwrap().range.as_deref(), Some("1:0"));
    assert!(top.signal("beat").is_some());
    assert_eq!(top.signal("beat").unwrap().range.as_deref(), Some("16:0"));
    assert_eq!(top.signal("a").unwrap().range.as_deref(), Some("7:0"));
    assert!(top.signal("b").is_some());
    // Interface instances are instances; interface ports are signals.
    assert!(top
        .instances
        .iter()
        .any(|inst| inst.module == "lb_if" && inst.name == "u_if"));
    assert!(top.signal("bus").is_some());
    assert!(!top.instances.iter().any(|inst| inst.name == "bus"));
    // `for (genvar i = 0; i < DEPTH; ...)` with an imported parameter.
    assert_eq!(top.body.len(), 2);
}

#[test]
fn package_parameters_and_imports_elaborate() {
    let dir = std::env::temp_dir().join(format!("waverdi_sv_pkg_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("sv.sv");
    fs::write(&file, SV_TYPES).unwrap();
    let set = SourceSet::from_files(vec![file], "test");
    let db = RtlDb::parse_sources(&set);
    // `pkg::NAME` is visible everywhere, `import pkg::*` binds plain names.
    let top = db.module("top").unwrap();
    let env = db.default_env(top);
    assert_eq!(env.get("cfg_pkg::WIDTH"), Some(&16));
    assert_eq!(env.get("WIDTH"), Some(&16));
    assert_eq!(env.get("DEPTH"), Some(&4));
    assert_eq!(env.get("N"), Some(&16));
    // The generate loop elaborates into g[0..3].u_sub.
    let placements = db.placements();
    let paths: Vec<String> = placements
        .iter()
        .map(|(steps, _)| steps.join("."))
        .collect();
    for index in 0..4 {
        assert!(
            paths
                .iter()
                .any(|path| path.ends_with(&format!("g[{index}].u_sub"))),
            "missing g[{index}].u_sub in {paths:?}"
        );
    }
}

/// Malformed inputs must terminate and keep whatever parsed before the
/// damage; the assertions pin the current partial-result behavior.
#[test]
fn malformed_modules_terminate_with_partial_results() {
    let path = Path::new("broken.sv");
    // Empty input and `endmodule` without a module: nothing to report.
    assert!(parse_module_text("", path).is_empty());
    assert!(parse_module_text("endmodule\n", path).is_empty());
    // A module truncated at EOF keeps its name and declarations.
    let mods = parse_module_text("module top;\n  logic a;\n", path);
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].name, "top");
    assert!(mods[0].signal("a").is_some());
    // `begin` without a matching `end` still yields the always block.
    let mods = parse_module_text("module top;\n  always @(*) begin\n    a = b;\n", path);
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].name, "top");
    assert_eq!(mods[0].always.len(), 1);
    // An unbalanced `(` does not lose the assignment.
    let mods = parse_module_text("module top;\n  assign y = (a + (b;\nendmodule\n", path);
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].assigns.len(), 1);
    assert_eq!(mods[0].assigns[0].lhs, "y");
}

#[test]
fn lexical_hazards_do_not_panic() {
    let path = Path::new("hazard.sv");
    // Garbage before a valid module is skipped by the lexer.
    let mods = parse_module_text("%%%\u{0}\u{ff}\u{1} module m(); endmodule\n", path);
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].name, "m");
    // An unclosed block comment swallows the rest of the file, but not
    // the module header seen before it.
    let mods = parse_module_text("module top;\n  /* never closed\n  logic a;\n", path);
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].name, "top");
    assert!(mods[0].signals.is_empty());
    // An unclosed string runs to EOF; the module still parses.
    let mods = parse_module_text("module top;\n  initial $display(\"abc\n", path);
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].name, "top");
}

#[test]
fn lexer_literals_and_escaped_identifiers() {
    assert_eq!(parse_int("8'hFF"), 255);
    assert_eq!(parse_int("'d125"), 125);
    // `'x` is an unsized all-x literal; without a width to expand it to,
    // the scanner falls back to 0.
    assert_eq!(parse_int("'x"), 0);
    // One `z` digit makes the whole literal unparsable, so it falls back
    // to 0 as well.
    assert_eq!(parse_int("10'b1z0"), 0);
    // An escaped identifier is one token; the leading backslash stays in
    // the name (the parser strips it where it matters).
    let tokens = lex("\\bus+clk ");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].tok, Tok::Ident("\\bus+clk".to_string()));
}

#[test]
fn eval_edge_cases() {
    let env = ParamEnv::new();
    let eval_text = |text: &str| parse_expr_text(text).and_then(|expr| eval(&expr, &env));

    // Division and modulo by zero are guarded to 0 instead of failing or
    // panicking.
    assert_eq!(eval_text("1/0"), Some(0));
    assert_eq!(eval_text("1%0"), Some(0));
    // Checked arithmetic: overflow refuses to wrap.
    assert_eq!(eval_text("9223372036854775807 + 1"), None);
    assert_eq!(eval_text("3037000500 * 3037000500"), None);
    // Ternary, shifts (an out-of-range shift is rejected), unary minus.
    assert_eq!(eval_text("5 ? 7 : 9"), Some(7));
    assert_eq!(eval_text("0 ? 7 : 9"), Some(9));
    assert_eq!(eval_text("1 << 3"), Some(8));
    assert_eq!(eval_text("8 >> 1"), Some(4));
    assert_eq!(eval_text("1 << 64"), None);
    assert_eq!(eval_text("-5"), Some(-5));
    // Unknown names evaluate to None.
    assert_eq!(eval_text("unknown_name"), None);
    assert_eq!(eval_text("unknown_name + 1"), None);
}
