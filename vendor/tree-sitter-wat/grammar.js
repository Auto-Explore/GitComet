/// <reference types="tree-sitter-cli/dsl" />

const PREC = {
  STRING: 2,
};

const pattern_dec_nat = /[0-9]+(_?[0-9]+)*/;
const pattern_hex_nat = /[0-9A-Fa-f]+(_?[0-9A-Fa-f]+)*/;
const pattern_identifier = /[0-9A-Za-z!#$%&'*+-./:<=>?@\\^_'|~]+/;
const pattern_sign = /[+-]/;

const imm = rule => token.immediate(rule);

module.exports = grammar({
  name: "wat",

  extras: $ => [$.annotation, $.comment_block, $.comment_line, /[\s\uFEFF\u2060\u200B\u00A0]/],

  conflicts: $ => [[$.op_let], [$.op_select]],

  rules: {
    ROOT: $ => choice($.module, repeat($.module_field)),

    align_value: $ => seq("align", imm("="), $.align_offset_value),

    align_offset_value: $ => imm(/[0-9]+(_?[0-9]+)*|0x[0-9A-Fa-f]+(_?[0-9A-Fa-f]+)*/),

    // proposal: annotations
    annotation: $ => seq("(@", $.identifier_pattern, repeat($.annotation_part), ")"),

    // proposal: annotations
    annotation_parens: $ => seq("(", repeat($.annotation_part), ")"),

    // proposal: annotations
    annotation_part: $ =>
      choice($.comment_block_annot, $.comment_line_annot, $.annotation_parens, $.reserved, $.identifier, $.string),

    block_block: $ =>
      seq(
        "block",
        optional($.identifier),
        seq(optional($.type_use), repeat($.func_type_params_many), repeat($.func_type_results), optional($.instr_list)),
        "end",
        optional($.identifier),
      ),

    block_if: $ =>
      seq(
        "if",
        optional($.identifier),
        seq(optional($.type_use), repeat($.func_type_params_many), repeat($.func_type_results), optional($.instr_list)),
        optional(seq("else", optional($.identifier), optional($.instr_list))),
        "end",
        optional($.identifier),
      ),

    block_loop: $ =>
      seq(
        "loop",
        optional($.identifier),
        seq(optional($.type_use), repeat($.func_type_params_many), repeat($.func_type_results), optional($.instr_list)),
        "end",
        optional($.identifier),
      ),

    comment_block: $ => seq("(;", repeat(choice($.comment_block, $.comment_block_inner)), ";)"),

    comment_block_annot: $ => seq("(;", repeat(choice($.comment_block_annot, $.comment_block_inner)), ";)"),

    comment_block_inner: $ => token(choice(/[^(;]+/, "(", ";")),

    comment_line: $ => prec.left(token(seq(";;", /.*/))),

    comment_line_annot: $ => prec.left(token(seq(";;", /.*/))),

    dec_float: $ =>
      token(
        seq(
          pattern_dec_nat,
          optional(seq(".", optional(pattern_dec_nat))),
          optional(seq(/[Ee]/, optional(pattern_sign), pattern_dec_nat)),
        ),
      ),

    dec_nat: $ => pattern_dec_nat,

    // proposal: reference-types
    elem_expr: $ => choice($.elem_expr_item, $.elem_expr_expr),

    // proposal: reference-types
    elem_expr_expr: $ => $.expr,

    // proposal: reference-types
    elem_expr_item: $ => seq("(", "item", repeat($.instr), ")"),

    // proposal: reference-types
    elem_kind: $ => "func",

    // proposal: reference-types
    elem_list: $ => choice(seq($.elem_kind, repeat($.index)), seq($.ref_type, repeat($.elem_expr))),

    // GitComet: `u{...}` has its braces escaped.
    //
    // Upstream wrote the Unicode escape as /u{[0-9a-fA-F]+}/, which the modern
    // tree-sitter CLI's regex engine reads as a repetition quantifier and
    // rejects -- so the grammar as committed could not be regenerated at all,
    // and the parser.c beside it was stale at ABI 13. The braces here are
    // literal, which is what upstream meant.
    escape_sequence: $ =>
      imm(repeat1(seq(imm("\\"), imm(choice(/[^u0-9a-fA-F]/, /[0-9a-fA-F]{2}/, /u\{[0-9a-fA-F]+\}/))))),

    export: $ => seq("(", "export", $.name, ")"),

    export_desc: $ => choice($.export_desc_func, $.export_desc_table, $.export_desc_memory, $.export_desc_global),

    export_desc_func: $ => seq("(", "func", $.index, ")"),

    export_desc_global: $ => seq("(", "global", $.index, ")"),

    export_desc_memory: $ => seq("(", "memory", $.index, ")"),

    export_desc_table: $ => seq("(", "table", $.index, ")"),

    expr: $ => seq("(", $.expr1, ")"),

    expr1: $ => choice($.expr1_plain, $.expr1_call, $.expr1_block, $.expr1_loop, $.expr1_if),

    expr1_block: $ =>
      seq(
        "block",
        optional($.identifier),
        seq(optional($.type_use), repeat($.func_type_params_many), repeat($.func_type_results), optional($.instr_list)),
      ),

    expr1_call: $ =>
      seq(
        "call_indirect",
        // proposal: reference-types
        optional($.index),
        optional($.type_use),
        repeat($.func_type_params_many),
        repeat($.func_type_results),
        repeat($.expr),
      ),

    expr1_if: $ => seq("if", optional($.identifier), $.if_block),

    expr1_loop: $ =>
      seq(
        "loop",
        optional($.identifier),
        seq(optional($.type_use), repeat($.func_type_params_many), repeat($.func_type_results), optional($.instr_list)),
      ),

    expr1_plain: $ => seq($.instr_plain, repeat($.expr)),

    float: $ => seq(optional($.sign), choice($.dec_float, $.hex_float, "inf", $.nan)),

    func_locals: $ => choice($.func_locals_one, $.func_locals_many),

    func_locals_many: $ => seq("(", "local", repeat($.value_type), ")"),

    func_locals_one: $ => seq("(", "local", $.identifier, $.value_type, ")"),

    func_type: $ => choice($.func_type_params, $.func_type_results),

    func_type_params: $ => choice($.func_type_params_one, $.func_type_params_many),

    func_type_params_many: $ => seq("(", "param", repeat($.value_type), ")"),

    func_type_params_one: $ => seq("(", "param", $.identifier, $.value_type, ")"),

    func_type_results: $ => seq("(", "result", repeat($.value_type), ")"),

    global_type: $ => choice($.global_type_imm, $.global_type_mut),

    global_type_imm: $ => $.value_type,

    global_type_mut: $ => seq("(", "mut", $.value_type, ")"),

    hex_float: $ =>
      token(
        seq(
          "0x",
          pattern_hex_nat,
          optional(seq(".", optional(pattern_hex_nat))),
          optional(seq(/[Pp]/, optional(pattern_sign), pattern_dec_nat)),
        ),
      ),

    hex_nat: $ => token(seq("0x", imm(pattern_hex_nat))),

    identifier: $ => token(seq("$", imm(pattern_identifier))),

    identifier_pattern: $ => imm(pattern_identifier),

    if_block: $ =>
      seq(
        optional($.type_use),
        repeat($.func_type_params_many),
        repeat($.func_type_results),
        repeat($.expr),
        seq("(", "then", optional($.instr_list), ")"),
        optional(seq("(", "else", optional($.instr_list), ")")),
      ),

    import: $ => seq("(", "import", $.name, $.name, ")"),

    import_desc: $ =>
      choice(
        $.import_desc_type_use,
        $.import_desc_func_type,
        $.import_desc_table_type,
        $.import_desc_memory_type,
        $.import_desc_global_type,
      ),

    import_desc_func_type: $ => seq("(", "func", optional($.identifier), repeat($.func_type), ")"),

    import_desc_global_type: $ => seq("(", "global", optional($.identifier), $.global_type, ")"),

    import_desc_memory_type: $ => seq("(", "memory", optional($.identifier), $.memory_type, ")"),

    import_desc_table_type: $ => seq("(", "table", optional($.identifier), $.table_type, ")"),

    import_desc_type_use: $ => seq("(", "func", optional($.identifier), $.type_use, ")"),

    index: $ => choice($.nat, $.identifier),

    instr: $ => choice($.instr_plain, $.instr_call, $.instr_block, $.expr),

    instr_block: $ => choice($.block_block, $.block_loop, $.block_if),

    instr_call: $ =>
      seq("call_indirect", optional($.type_use), repeat($.func_type_params_many), repeat($.func_type_results), $.instr),

    // NOTE: this must be wrapped in "optional"
    instr_list: $ => repeat1(choice($.instr_list_call, $.instr)),

    instr_list_call: $ =>
      prec.right(
        seq("call_indirect", optional($.type_use), repeat($.func_type_params_many), repeat($.func_type_results)),
      ),

    instr_plain: $ =>
      choice(
        $.op_nullary,
        seq($.op_index, $.index),
        seq($.op_index_opt, optional($.index)),
        seq("br_table", $.index, repeat($.index)),
        seq("ref.extern", $.nat),
        seq("ref.null", choice($.ref_kind, $.index)),
        seq($.op_index_opt_offset_opt_align_opt, optional($.index), optional($.offset_value), optional($.align_value)),
        seq($.op_simd_offset_opt_align_opt, optional($.offset_value), optional($.align_value)),
        $.op_const,
        $.op_func_bind,
        $.op_let,
        $.op_select,
        $.op_simd_const,
        $.op_simd_lane,
        $.op_table_copy,
        $.op_table_init,
      ),

    op_nullary: $ =>
      token(
        choice(
          "atomic.fence",
          seq(
            "f",
            choice(
              seq(
                imm("32"),
                choice(
                  seq(
                    imm("x4."),
                    new RegExp(
                      [
                        "add",
                        "convert_i32x4_[su]",
                        "div",
                        "eq",
                        "g[et]",
                        "l[et]",
                        "mul",
                        "ne",
                        "p?(m(ax|in))",
                        "s(qrt|ub)",
                      ].join("|"),
                    ),
                  ),
                  seq(
                    imm("."),
                    new RegExp(
                      [
                        "a(bs|dd)",
                        "c(eil|o(nvert_i(32|64)_[su]|pysign))",
                        "d(emote_f64|iv)",
                        "eqz?",
                        "floor",
                        "g[et]",
                        "l[et]",
                        "m(ax|in|ul)",
                        "ne(arest|g)?",
                        "reinterpret_i32",
                        "s(qrt|ub)",
                        "trunc",
                      ].join("|"),
                    ),
                  ),
                ),
              ),
              seq(
                imm("64"),
                choice(
                  seq(
                    imm("x2."),
                    new RegExp(
                      ["add", "div", "eq", "g[et]", "l[et]", "mul", "ne", "p?(m(ax|in))", "s(qrt|ub)"].join("|"),
                    ),
                  ),
                  seq(
                    imm("."),
                    new RegExp(
                      [
                        "a(bs|dd)",
                        "c(eil|o(nvert_i(32|64)_[su]|pysign))",
                        "div",
                        "eqz?",
                        "floor",
                        "g[et]",
                        "l[et]",
                        "m(ax|in|ul)",
                        "ne(arest|g)?",
                        "promote_f32",
                        "reinterpret_i64",
                        "s(qrt|ub)",
                        "trunc",
                      ].join("|"),
                    ),
                  ),
                ),
              ),
            ),
          ),
          seq(
            "i",
            choice(
              seq(
                imm("8x16."),
                imm(
                  new RegExp(
                    [
                      "((add|sub)_sat|avgr|m(ax|in)|shr)_[su]",
                      "add",
                      "eq",
                      "[gl][et]_[su]",
                      "mul",
                      "n(arrow_i16x8_[su]|e)",
                      "s(hl|ub|wizzle)",
                    ].join("|"),
                  ),
                ),
              ),
              seq(
                imm("16x8."),
                imm(
                  new RegExp(
                    [
                      "((add|sub)_sat|avgr|m(ax|in)|shr)_[su]",
                      "add",
                      "eq",
                      "[gl][et]_[su]",
                      "mul",
                      "n(arrow_i32x4_[su]|e)",
                      "s(hl|ub)",
                      "widen_(high|low)_i8x16_[su]",
                    ].join("|"),
                  ),
                ),
              ),
              seq(
                imm("32x4."),
                imm(
                  new RegExp(
                    [
                      "((add|sub)_sat|avgr|m(ax|in)|shr)_[su]",
                      "add",
                      "eq",
                      "[gl][et]_[su]",
                      "mul",
                      "ne",
                      "s(hl|ub)",
                      "trunc_sat_f32x4_[su]",
                      "widen_(high|low)_i16x8_[su]",
                    ].join("|"),
                  ),
                ),
              ),
              seq(imm("64x2."), imm(new RegExp(["add", "mul", "s(h(l|r_[su])|ub)"].join("|")))),
              seq(
                imm("32."),
                choice(
                  seq(
                    imm("atomic."),
                    choice(
                      new RegExp(["load((8|16)_u)?", "store(8|16)?"].join("|")),
                      seq(
                        imm("rmw"),
                        choice(
                          seq(imm("."), new RegExp(["a(dd|nd)", "cmpxchg", "or", "sub", "x(chg|or)"].join("|"))),
                          seq(
                            imm(/(8|16)\./),
                            new RegExp(["a(dd|nd)", "cmpxchg", "or", "sub", "x(chg|or)"].join("|")),
                            imm("_u"),
                          ),
                        ),
                      ),
                    ),
                  ),
                  new RegExp(
                    [
                      "a(nd|dd)",
                      "c[lt]z",
                      "(div|g[et]|l[et]|rem|shr)_[su]",
                      "e(qz?|xtend(8|16)_s)",
                      "mul",
                      "ne",
                      "or",
                      "popcnt",
                      "r(einterpret_f32|ot[lr])",
                      "s(hl|ub)",
                      "trunc(_sat)?_f(32|64)_[su]",
                      "wrap_i64",
                      "xor",
                    ].join("|"),
                  ),
                ),
              ),
              seq(
                imm("64."),
                choice(
                  seq(
                    imm("atomic."),
                    choice(
                      new RegExp(["load((8|16|32)_u)?", "store(8|16|32)?"].join("|")),
                      seq(
                        imm("rmw"),
                        choice(
                          seq(imm("."), new RegExp(["a(dd|nd)", "cmpxchg", "or", "sub", "x(chg|or)"].join("|"))),
                          seq(
                            imm(/(8|16|32)\./),
                            new RegExp(["a(dd|nd)", "cmpxchg", "or", "sub", "x(chg|or)"].join("|")),
                            imm("_u"),
                          ),
                        ),
                      ),
                    ),
                  ),
                  new RegExp(
                    [
                      "a(nd|dd)",
                      "c[lt]z",
                      "(div|g[et]|l[et]|rem|shr)_[su]",
                      "e(qz?|xtend(_i32_[su]|(8|16|32)_s))",
                      "mul",
                      "ne",
                      "or",
                      "rot[lr]",
                      "popcnt",
                      "reinterpret_f64",
                      "s(hl|ub)",
                      "trunc(_sat)?_f(32|64)_[su]",
                      "xor",
                    ].join("|"),
                  ),
                ),
              ),
            ),
          ),
          new RegExp(
            [
              "call_ref",
              "drop",
              "memory\\.(atomic\\.(notify|wait(32|64))|copy|fill)",
              "nop",
              "re(f\\.(as_non_null|is_null)|turn(_call_ref)?)",
              "unreachable",
            ].join("|"),
          ),
          seq(imm("v128."), imm(/and(not)?|bitselect|not|x?or/)),
        ),
      ),

    op_index: $ =>
      token(
        new RegExp(
          [
            "br(_(if|on_null))?",
            "call",
            "data\\.drop",
            "elem\\.drop",
            "global\\.(get|set)",
            "local\\.(get|set|tee)",
            "memory\\.init",
            "ref\\.func",
          ].join("|"),
        ),
      ),

    op_index_opt: $ =>
      token(
        new RegExp(["local\\.(get|set|tee)", "memory\\.(grow|size)", "table\\.(fill|g(et|row)|s(et|ize))"].join("|")),
      ),

    op_index_opt_offset_opt_align_opt: $ =>
      token(
        choice(
          /f(32|64)\.(load|store)/,
          /i32\.(load((8|16)_[su])?|store(8|16)?)/,
          /i64\.(load((8|16|32)_[su])?|store(8|16|32)?)/,
        ),
      ),

    op_simd_offset_opt_align_opt: $ =>
      token(
        choice(
          /f(32x4|64x2)\.(abs|ceil|floor|nearest|neg|splat|trunc)/,
          seq(
            "i",
            imm(
              choice(
                /8x16\.(neg|splat|a(bs|ll_true|ny_true)|bitmask)/,
                /16x8\.(neg|splat|a(bs|ll_true|ny_true)|bitmask|load8x8_[su])/,
                /32x4\.(neg|splat|a(bs|ll_true|ny_true)|bitmask|load16x4_[su])/,
                /64x2\.(neg|splat|load32x2_[su])/,
              ),
            ),
          ),
          seq(
            "v",
            choice(
              imm(/(8x16|16x8|32x4|64x2)\.load_splat/),
              imm(/128\.(load((8|16|32|64)_splat|(8x8|16x4|32x2)_[su])?|store)/),
            ),
          ),
        ),
      ),

    op_const: $ =>
      choice(seq(alias(/f(32|64)\.const/, $.pat00), $.float), seq(alias(/i(32|64)\.const/, $.pat01), $.int)),

    // proposal: function-references
    op_func_bind: $ => prec.right(seq("func.bind", optional(seq("(", "type", $.index, ")")))),

    // proposal: function-references
    op_let: $ =>
      seq("let", optional($.index), repeat($.func_type_params), repeat($.func_type_results), repeat($.func_locals)),

    op_select: $ =>
      seq(
        "select",
        // proposal: reference-types
        repeat($.func_type_results),
      ),

    // proposal: simd
    op_simd_const: $ =>
      seq(
        "v128.const",
        choice(
          seq("f32x4", ...Array(4).fill($.float)),
          seq("f64x2", ...Array(2).fill($.float)),
          seq("i8x16", ...Array(16).fill($.int)),
          seq("i16x8", ...Array(8).fill($.int)),
          seq("i32x4", ...Array(4).fill($.int)),
          seq("i64x2", ...Array(2).fill($.int)),
        ),
      ),

    // proposal: simd
    op_simd_lane: $ =>
      choice(
        seq(seq("i8x16", imm(".shuffle")), ...Array(16).fill($.float)),
        seq(seq(choice("i8x16", "i16x8"), imm("."), imm("extract_lane"), imm("_"), imm(alias(/[su]/, $.pat00))), $.int),
        seq(seq(choice("f32x4", "f64x2", "i32x4", "i64x2"), imm("."), imm("extract_lane")), $.int),
        seq(seq(choice("f32x4", "f64x2", "i8x16", "i16x8", "i32x4", "i64x2"), imm("."), imm("replace_lane")), $.int),
      ),

    // proposal: bulk-memory-operations
    op_table_copy: $ => seq("table.copy", optional(seq($.index, $.index))),

    // proposal: bulk-memory-operations
    op_table_init: $ => seq("table.init", $.index, optional($.index)),

    int: $ => seq(optional($.sign), $.nat),

    limits: $ =>
      seq(
        $.nat,
        optional($.nat),
        // proposal: threads
        optional($.share),
      ),

    memory_fields_data: $ => seq("(", "data", repeat($.string), ")"),

    memory_fields_type: $ => seq(optional($.import), $.memory_type),

    memory_type: $ => $.limits,

    memory_use: $ => seq("(", "memory", $.index, ")"),

    module: $ => seq("(", "module", optional(field("identifier", $.identifier)), repeat($.module_field), ")"),

    module_field: $ =>
      choice(
        $.module_field_type,
        $.module_field_global,
        $.module_field_table,
        $.module_field_memory,
        $.module_field_func,
        $.module_field_elem,
        $.module_field_data,
        $.module_field_start,
        $.module_field_import,
        $.module_field_export,
      ),

    module_field_data: $ =>
      seq("(", "data", optional($.index), optional(seq(optional($.memory_use), $.offset)), repeat($.string), ")"),

    module_field_elem: $ =>
      seq(
        "(",
        "elem",
        optional($.index),
        choice(
          $.elem_list,
          seq(
            $.table_use,
            $.offset,
            choice(
              $.elem_list,
              // proposal: multi-memory
              repeat($.index),
            ),
          ),
          seq("declare", $.elem_list),
          seq($.offset, $.elem_list),
          seq($.offset, repeat($.index)),
        ),
        ")",
      ),

    module_field_export: $ => seq("(", "export", $.name, $.export_desc, ")"),

    module_field_func: $ =>
      seq(
        "(",
        "func",
        optional(field("identifier", $.identifier)),
        repeat($.export),
        optional($.import),
        optional($.type_use),
        repeat($.func_type_params),
        repeat($.func_type_results),
        repeat($.func_locals),
        optional($.instr_list),
        ")",
      ),

    module_field_global: $ =>
      seq(
        "(",
        "global",
        optional(field("identifier", $.identifier)),
        repeat($.export),
        optional($.import),
        $.global_type,
        repeat($.instr),
        ")",
      ),

    module_field_import: $ => seq("(", "import", $.name, $.name, $.import_desc, ")"),

    module_field_memory: $ =>
      seq(
        "(",
        "memory",
        optional(field("identifier", $.identifier)),
        repeat($.export),
        choice($.memory_fields_data, $.memory_fields_type),
        ")",
      ),

    module_field_start: $ => seq("(", "start", $.index, ")"),

    module_field_table: $ =>
      seq(
        "(",
        "table",
        optional(field("identifier", $.identifier)),
        repeat($.export),
        choice($.table_fields_elem, $.table_fields_type),
        ")",
      ),

    module_field_type: $ => seq("(", "type", optional(field("identifier", $.identifier)), $.type_field, ")"),

    name: $ => $.string,

    nan: $ =>
      token(
        seq(
          "nan",
          optional(seq(imm(":"), choice(imm("arithmetic"), imm("canonical"), seq(imm("0x"), imm(pattern_hex_nat))))),
        ),
      ),

    nat: $ => choice($.dec_nat, $.hex_nat),

    num: $ => choice($.int, $.float),

    num_type_f32: $ => "f32",

    num_type_f64: $ => "f64",

    num_type_i32: $ => "i32",

    num_type_i64: $ => "i64",

    num_type_v128: $ => "v128",

    offset: $ => choice($.offset_const_expr, $.offset_expr),

    offset_const_expr: $ => seq("(", "offset", repeat($.instr), ")"),

    offset_expr: $ => $.expr,

    offset_value: $ => seq("offset", imm("="), $.align_offset_value),

    // proposal: reference-types
    ref_kind: $ => /extern|func/,

    // proposal: reference-types
    ref_type: $ => choice($.ref_type_externref, $.ref_type_funcref, $.ref_type_ref),

    // proposal: reference-types
    ref_type_externref: $ => "externref",

    // proposal: reference-types
    ref_type_funcref: $ => "funcref",

    // proposal: function-references
    ref_type_ref: $ => seq("(", "ref", optional("null"), choice($.ref_kind, $.index), ")"),

    // eslint-disable-next-line no-useless-escape
    reserved: $ => token(choice(pattern_identifier, /[,;\[\]{}]/)),

    // proposal: threads
    share: $ => /(un)?shared/,

    sign: $ => /[+-]/,

    string: $ => seq('"', repeat(choice(imm(prec(PREC.STRING, /[^"\\\n]+|\\\r?\n/)), $.escape_sequence)), '"'),

    table_fields_elem: $ =>
      seq($.ref_type, "(", "elem", choice(repeat($.index), seq($.elem_expr, repeat($.elem_expr))), ")"),

    table_fields_type: $ => seq(optional($.import), $.table_type),

    table_type: $ => seq($.limits, $.ref_type),

    table_use: $ => seq("(", "table", $.index, ")"),

    type_field: $ => seq("(", "func", repeat($.func_type), ")"),

    type_use: $ => seq("(", "type", $.index, ")"),

    value_type: $ => choice($.value_type_num_type, $.value_type_ref_type),

    value_type_num_type: $ => choice($.num_type_f32, $.num_type_f64, $.num_type_i32, $.num_type_i64, $.num_type_v128),

    value_type_ref_type: $ => $.ref_type,
  },
});
