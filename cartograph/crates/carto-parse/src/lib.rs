//! `carto-parse` — tree-sitter extraction (symbols, skeletons, refs, purpose).
//!
//! Vertical slice: TypeScript (`.ts` / `.tsx` / `.mts` / `.cts`). Uses the
//! tree-sitter TypeScript grammar to walk a single file and emit a
//! [`ParsedFile`]: a flat list of [`RawSymbol`]s (with `parent_idx` nesting),
//! a list of [`RawRef`]s (calls / imports / type-refs keyed by name), and a
//! file-level `purpose` line. No LLM, no resolution — refs carry only names and
//! resolution to concrete defs happens later in H2 (`carto-graph`).
//!
//! Resolved version pair (empirically): `tree-sitter 0.22.6` +
//! `tree-sitter-typescript 0.21.2`, which exposes `language_typescript()` /
//! `language_tsx()` returning a `tree_sitter::Language`.

use carto_model::{stable_key, EdgeKind, ParsedFile, RawRef, RawSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

/// Detect language by file extension. Returns the tree-sitter grammar id.
pub fn detect_lang(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "ts" | "tsx" | "mts" | "cts" => Some("typescript".to_string()),
        _ => None,
    }
}

/// Parse one TypeScript file into symbols / refs / purpose.
///
/// `path` is repo-relative (used to build `stable_key`); `src` is the file
/// text. On unknown language or any parse failure this returns
/// [`ParsedFile::default`] (empty) rather than panicking.
pub fn parse_file(path: &str, src: &str) -> ParsedFile {
    let lang = match detect_lang(path) {
        Some(l) => l,
        None => return ParsedFile::default(),
    };

    // `.tsx` uses the TSX dialect (JSX-aware); everything else uses the plain
    // TypeScript grammar.
    let is_tsx = path.rsplit('.').next() == Some("tsx");
    let grammar = if is_tsx {
        tree_sitter_typescript::language_tsx()
    } else {
        tree_sitter_typescript::language_typescript()
    };

    let mut parser = Parser::new();
    if parser.set_language(&grammar).is_err() {
        return ParsedFile::default();
    }
    let tree = match parser.parse(src, None) {
        Some(t) => t,
        None => return ParsedFile::default(),
    };

    let bytes = src.as_bytes();
    let mut ctx = Extractor {
        path,
        bytes,
        symbols: Vec::new(),
        refs: Vec::new(),
    };
    let root = tree.root_node();
    ctx.walk_container(root, None, &[]);
    // Top-level imports have no enclosing symbol. Rather than dropping them
    // (they're load-bearing for the H2 import map), attribute them to the first
    // top-level symbol in the file. If the file defines no symbols, they're
    // skipped (documented v1 limitation).
    ctx.collect_toplevel_imports(root);

    let purpose = file_purpose(root, bytes);

    ParsedFile {
        lang: Some(lang),
        purpose,
        symbols: ctx.symbols,
        refs: ctx.refs,
    }
}

/// Mutable extraction state threaded through the tree walk.
struct Extractor<'a> {
    path: &'a str,
    bytes: &'a [u8],
    symbols: Vec<RawSymbol>,
    refs: Vec<RawRef>,
}

impl<'a> Extractor<'a> {
    /// Slice node source text.
    fn text(&self, node: Node) -> &'a str {
        node.utf8_text(self.bytes).unwrap_or("")
    }

    /// Walk the children of a container node (program or class body), emitting
    /// symbols. `parent_idx` is the enclosing symbol (e.g. a class for its
    /// methods); `container_path` is the slash-joined chain of enclosing names
    /// for `stable_key`.
    fn walk_container(&mut self, node: Node, parent_idx: Option<usize>, container_path: &[&str]) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            // An `export`/`declare` wrapper just forwards to its inner decl.
            let decl = match child.kind() {
                "export_statement" => match inner_declaration(child) {
                    Some(d) => d,
                    None => {
                        // `export { ... }` re-export with no declaration: still
                        // scan it for refs (imports surface as identifiers).
                        self.scan_refs(child, parent_idx);
                        continue;
                    }
                },
                _ => child,
            };
            self.handle_decl(decl, parent_idx, container_path);
        }
    }

    /// Dispatch a single (possibly export-unwrapped) declaration node.
    fn handle_decl(&mut self, decl: Node, parent_idx: Option<usize>, container_path: &[&str]) {
        match decl.kind() {
            "function_declaration" | "generator_function_declaration" => {
                let idx = self.emit_named(
                    decl,
                    "name",
                    SymbolKind::Function,
                    parent_idx,
                    container_path,
                );
                if let Some(i) = idx {
                    self.scan_refs(decl, Some(i));
                }
            }
            "class_declaration" | "abstract_class_declaration" => {
                self.handle_class(decl, parent_idx, container_path);
            }
            "interface_declaration" => {
                let idx = self.emit_named(
                    decl,
                    "name",
                    SymbolKind::Interface,
                    parent_idx,
                    container_path,
                );
                if let Some(i) = idx {
                    self.scan_type_refs_in(decl, Some(i));
                }
            }
            "type_alias_declaration" => {
                let idx =
                    self.emit_named(decl, "name", SymbolKind::Type, parent_idx, container_path);
                if let Some(i) = idx {
                    self.scan_type_refs_in(decl, Some(i));
                }
            }
            // `const x = ...`, `let x = ...`, `var x = ...` — one symbol per
            // declarator. An arrow/function value is classified as Function;
            // any other value is a Const.
            "lexical_declaration" | "variable_declaration" => {
                self.handle_variable(decl, parent_idx, container_path);
            }
            // Class members.
            "method_definition" => {
                let idx =
                    self.emit_named(decl, "name", SymbolKind::Method, parent_idx, container_path);
                if let Some(i) = idx {
                    self.scan_refs(decl, Some(i));
                }
            }
            "public_field_definition" => {
                let idx =
                    self.emit_named(decl, "name", SymbolKind::Field, parent_idx, container_path);
                // Field initializers can carry calls/type-refs.
                if let Some(i) = idx {
                    self.scan_refs(decl, Some(i));
                }
            }
            _ => {
                // Anything else at this level (statements, comments). Attribute
                // any refs inside to the enclosing symbol (None => skipped).
                self.scan_refs(decl, parent_idx);
            }
        }
    }

    /// Emit a class symbol, then recurse into its body for methods/fields and
    /// capture extends/implements as Inherit/Implement edges.
    fn handle_class(&mut self, decl: Node, parent_idx: Option<usize>, container_path: &[&str]) {
        let idx = match self.emit_named(decl, "name", SymbolKind::Class, parent_idx, container_path)
        {
            Some(i) => i,
            None => return,
        };

        // Heritage: `extends Base` -> Inherit, `implements Iface` -> Implement.
        if let Some(heritage) = child_of_kind(decl, "class_heritage") {
            let mut hc = heritage.walk();
            for clause in heritage.named_children(&mut hc) {
                let kind = match clause.kind() {
                    "extends_clause" => EdgeKind::Inherit,
                    "implements_clause" => EdgeKind::Implement,
                    _ => continue,
                };
                let mut cc = clause.walk();
                for t in clause.named_children(&mut cc) {
                    if let Some(name) = type_ref_name(t, self.bytes) {
                        self.refs.push(RawRef {
                            src_idx: idx,
                            target_name: name,
                            kind,
                        });
                    }
                }
            }
        }

        // Recurse into the class body. The class name extends the container
        // path so nested methods get `Class/method` stable keys.
        if let Some(body) = child_of_kind(decl, "class_body") {
            let name = self.symbols[idx].name.clone();
            let mut new_path: Vec<&str> = container_path.to_vec();
            new_path.push(&name);
            self.walk_container(body, Some(idx), &new_path);
        }
    }

    /// Handle `const`/`let`/`var` — emit one symbol per declarator.
    fn handle_variable(&mut self, decl: Node, parent_idx: Option<usize>, container_path: &[&str]) {
        let mut cursor = decl.walk();
        for declarator in decl.named_children(&mut cursor) {
            if declarator.kind() != "variable_declarator" {
                continue;
            }
            let name_node = match declarator.child_by_field_name("name") {
                Some(n) => n,
                None => continue,
            };
            // Only handle simple identifier bindings (skip destructuring).
            if name_node.kind() != "identifier" {
                continue;
            }
            let value = declarator.child_by_field_name("value");
            let is_fn = value.is_some_and(|v| {
                matches!(
                    v.kind(),
                    "arrow_function" | "function" | "function_expression"
                )
            });
            let kind = if is_fn {
                SymbolKind::Function
            } else {
                SymbolKind::Const
            };

            let name = self.text(name_node).to_string();
            // Signature spans the whole declaration up to the value body.
            let sig = variable_signature(decl, declarator, value, self.bytes);
            let idx = self.push_symbol(decl, name, kind, sig, parent_idx, container_path);
            // Refs inside the initializer attribute to this symbol.
            self.scan_refs(declarator, Some(idx));
        }
    }

    /// Emit a symbol whose name is at the given field, returning its index.
    fn emit_named(
        &mut self,
        decl: Node,
        name_field: &str,
        kind: SymbolKind,
        parent_idx: Option<usize>,
        container_path: &[&str],
    ) -> Option<usize> {
        let name_node = decl.child_by_field_name(name_field)?;
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return None;
        }
        let sig = signature_text(decl, self.bytes);
        Some(self.push_symbol(decl, name, kind, sig, parent_idx, container_path))
    }

    /// Push a [`RawSymbol`] with computed stable_key/fqn/doc/span.
    fn push_symbol(
        &mut self,
        decl: Node,
        name: String,
        kind: SymbolKind,
        signature: Option<String>,
        parent_idx: Option<usize>,
        container_path: &[&str],
    ) -> usize {
        let key = stable_key(self.path, container_path, &name, kind);
        let fqn = if container_path.is_empty() {
            None
        } else {
            Some(format!("{}.{}", container_path.join("."), name))
        };
        let doc = leading_doc(decl, self.bytes);
        let sym = RawSymbol {
            stable_key: key,
            name,
            fqn,
            kind,
            signature,
            doc,
            start_byte: decl.start_byte() as i64,
            end_byte: decl.end_byte() as i64,
            start_row: decl.start_position().row as i64,
            end_row: decl.end_position().row as i64,
            parent_idx,
        };
        self.symbols.push(sym);
        self.symbols.len() - 1
    }

    /// Recursively scan a subtree for reference sites (calls / imports /
    /// type-refs) attributing each to `src_idx`. If `src_idx` is `None` the ref
    /// has no enclosing symbol (file top level) and is skipped — a documented
    /// v1 limitation.
    fn scan_refs(&mut self, node: Node, src_idx: Option<usize>) {
        match node.kind() {
            "import_statement" => {
                self.collect_imports(node, src_idx);
                return; // don't descend further into the import
            }
            "call_expression" => {
                if let Some(idx) = src_idx {
                    if let Some(name) = call_target_name(node, self.bytes) {
                        self.refs.push(RawRef {
                            src_idx: idx,
                            target_name: name,
                            kind: EdgeKind::Call,
                        });
                    }
                }
                // Fall through to descend into arguments (nested calls etc).
            }
            "type_annotation" | "type_arguments" | "type_parameters" => {
                if let Some(idx) = src_idx {
                    self.scan_type_refs_in(node, Some(idx));
                }
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.scan_refs(child, src_idx);
        }
    }

    /// Collect `import ... from "x"` bindings as Import refs (one per imported
    /// name; the original name is preferred over a local alias).
    fn collect_imports(&mut self, node: Node, src_idx: Option<usize>) {
        let idx = match src_idx {
            Some(i) => i,
            None => return, // top-level import with no enclosing symbol: skip
        };
        if let Some(clause) = child_of_kind(node, "import_clause") {
            let mut cursor = clause.walk();
            for c in clause.named_children(&mut cursor) {
                match c.kind() {
                    // default import: `import Def from "x"`
                    "identifier" => self.push_import(idx, &c),
                    // namespace import: `import * as ns from "x"`
                    "namespace_import" => {
                        if let Some(id) = child_of_kind(c, "identifier") {
                            self.push_import(idx, &id);
                        }
                    }
                    // named imports: `import { a, b as c } from "x"`
                    "named_imports" => {
                        let mut nc = c.walk();
                        for spec in c.named_children(&mut nc) {
                            if spec.kind() == "import_specifier" {
                                // Prefer the original `name:` field (the thing
                                // being imported), not the local alias.
                                let n = spec
                                    .child_by_field_name("name")
                                    .or_else(|| child_of_kind(spec, "identifier"));
                                if let Some(n) = n {
                                    self.push_import(idx, &n);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Attribute all top-level `import` statements to the first top-level
    /// symbol in the file (the file's import map owner). Skipped entirely if the
    /// file defines no top-level symbols.
    fn collect_toplevel_imports(&mut self, root: Node) {
        let first_top = self.symbols.iter().position(|s| s.parent_idx.is_none());
        let owner = match first_top {
            Some(i) => i,
            None => return,
        };
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if child.kind() == "import_statement" {
                self.collect_imports(child, Some(owner));
            }
        }
    }

    fn push_import(&mut self, src_idx: usize, name_node: &Node) {
        let name = self.text(*name_node).to_string();
        if !name.is_empty() {
            self.refs.push(RawRef {
                src_idx,
                target_name: name,
                kind: EdgeKind::Import,
            });
        }
    }

    /// Scan a subtree for type identifiers, attributing each as a TypeRef.
    fn scan_type_refs_in(&mut self, node: Node, src_idx: Option<usize>) {
        let idx = match src_idx {
            Some(i) => i,
            None => return,
        };
        if let Some(name) = type_ref_name(node, self.bytes) {
            self.refs.push(RawRef {
                src_idx: idx,
                target_name: name,
                kind: EdgeKind::TypeRef,
            });
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.scan_type_refs_in(child, src_idx);
        }
    }
}

// ── free helpers ────────────────────────────────────────────────────────────

/// Unwrap the declaration inside an `export_statement` (skipping `export`,
/// `default`, `declare` keywords).
fn inner_declaration(export: Node) -> Option<Node> {
    if let Some(d) = export.child_by_field_name("declaration") {
        return Some(d);
    }
    let mut cursor = export.walk();
    for child in export.named_children(&mut cursor) {
        match child.kind() {
            "function_declaration"
            | "generator_function_declaration"
            | "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "lexical_declaration"
            | "variable_declaration" => return Some(child),
            _ => {}
        }
    }
    None
}

/// First named child of a given kind.
fn child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// Extract a type identifier name from a type node (for extends/implements/
/// annotations). Returns the rightmost segment for qualified names
/// (`a.b.Type` -> `Type`).
fn type_ref_name(node: Node, bytes: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" | "identifier" => node.utf8_text(bytes).ok().map(str::to_string),
        // `generic_type` wraps `name` + type_arguments.
        "generic_type" => {
            if let Some(n) = node.child_by_field_name("name") {
                type_ref_name(n, bytes)
            } else {
                child_of_kind(node, "type_identifier").and_then(|t| type_ref_name(t, bytes))
            }
        }
        // qualified `nested_type_identifier` -> last segment.
        "nested_type_identifier" => child_of_kind(node, "type_identifier")
            .and_then(|t| t.utf8_text(bytes).ok().map(str::to_string)),
        _ => None,
    }
}

/// The callee name of a call expression. For `a.b.foo()` returns `foo`
/// (last member segment); for `foo()` returns `foo`.
fn call_target_name(call: Node, bytes: &[u8]) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    match func.kind() {
        "identifier" => func.utf8_text(bytes).ok().map(str::to_string),
        "member_expression" => func
            .child_by_field_name("property")
            .and_then(|p| p.utf8_text(bytes).ok())
            .map(str::to_string),
        _ => None,
    }
}

/// Render a declaration's signature: the header text from the symbol start up
/// to (but excluding) the body. Trimmed and collapsed to a single line.
fn signature_text(decl: Node, bytes: &[u8]) -> Option<String> {
    let start = decl.start_byte();
    // Cut before the first "body" node we recognize.
    let body = first_body_node(decl);
    let end = body.map(|b| b.start_byte()).unwrap_or(decl.end_byte());
    slice_signature(bytes, start, end)
}

/// The body delimiter node for a declaration (the part to strip for a
/// signature): a statement block, class/interface body.
fn first_body_node(decl: Node) -> Option<Node> {
    let mut cursor = decl.walk();
    for child in decl.children(&mut cursor) {
        match child.kind() {
            "statement_block" | "class_body" | "interface_body" => return Some(child),
            _ => {}
        }
    }
    None
}

/// Build a signature for a variable/const declaration: from the keyword through
/// the binding + type annotation, stopping at the value (or at the arrow body
/// for arrow functions, keeping the `=>`).
fn variable_signature(
    decl: Node,
    declarator: Node,
    value: Option<Node>,
    bytes: &[u8],
) -> Option<String> {
    let start = decl.start_byte();
    let end = match value {
        // For an arrow fn, keep the param list + return type and the `=>`, but
        // drop the body: slice up to the arrow body start.
        Some(v) if v.kind() == "arrow_function" => {
            arrow_body_start(v).unwrap_or_else(|| v.end_byte())
        }
        // Other values: stop at the value start (keeps `const x: T`).
        Some(v) => v.start_byte(),
        None => declarator.end_byte(),
    };
    slice_signature(bytes, start, end)
}

/// Byte offset of an arrow function's body (after `=>`), so we can keep the
/// param list + return annotation + `=>` in the signature.
fn arrow_body_start(arrow: Node) -> Option<usize> {
    if let Some(b) = arrow.child_by_field_name("body") {
        return Some(b.start_byte());
    }
    let mut cursor = arrow.walk();
    let mut seen_arrow = false;
    for child in arrow.children(&mut cursor) {
        if seen_arrow {
            return Some(child.start_byte());
        }
        if child.kind() == "=>" {
            seen_arrow = true;
        }
    }
    None
}

/// Slice `bytes[start..end]`, trim, collapse internal whitespace to single
/// spaces, and strip a trailing `=`/`{`. Returns `None` if empty.
fn slice_signature(bytes: &[u8], start: usize, end: usize) -> Option<String> {
    if end <= start || end > bytes.len() {
        return None;
    }
    let raw = std::str::from_utf8(&bytes[start..end]).ok()?;
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim().trim_end_matches(['{', ' ']).trim();
    // Keep a trailing `=>` (arrow signatures) but drop a bare trailing `=`.
    let trimmed = if trimmed.ends_with("=>") {
        trimmed
    } else {
        trimmed.trim_end_matches('=').trim()
    };
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// The immediately-preceding comment for a declaration, trimmed. Handles both
/// `/** ... */` block/JSDoc comments and `//` line comments. Returns `None` if
/// the previous sibling is not a comment adjacent to `decl`.
fn leading_doc(decl: Node, bytes: &[u8]) -> Option<String> {
    // For exported decls the comment sits before the `export_statement`, so
    // climb to the outermost wrapper before looking at the previous sibling.
    let mut anchor = decl;
    while let Some(parent) = anchor.parent() {
        if parent.kind() == "export_statement" {
            anchor = parent;
        } else {
            break;
        }
    }

    let prev = anchor.prev_sibling()?;
    if prev.kind() != "comment" {
        return None;
    }
    // Must be adjacent (no blank line of code between). Allow at most a 1-line
    // gap (the comment line immediately above the declaration).
    if anchor.start_position().row > prev.end_position().row + 1 {
        return None;
    }
    let text = prev.utf8_text(bytes).ok()?;
    let cleaned = clean_comment(text);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Strip comment markers (`//`, `/*`, `*/`, leading `*`) and trim.
fn clean_comment(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix("/*") {
        let inner = inner.strip_suffix("*/").unwrap_or(inner);
        inner
            .lines()
            .map(|l| l.trim().trim_start_matches('*').trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        // One or more `//` lines (a single comment node here).
        raw.lines()
            .map(|l| l.trim().trim_start_matches("//").trim())
            .collect::<Vec<_>>()
            .join(" ")
    }
    .trim()
    .to_string()
}

/// Derive a one-line file purpose from a leading block/JSDoc or banner `//`
/// comment at the top of the file. First sentence/line, trimmed, <=120 chars.
/// Filters obvious license/shebang headers.
fn file_purpose(root: Node, bytes: &[u8]) -> Option<String> {
    let first = root.named_child(0)?;
    if first.kind() != "comment" {
        return None;
    }
    // Comment must start at the very top of the file (row 0 or 1).
    if first.start_position().row > 1 {
        return None;
    }
    let raw = first.utf8_text(bytes).ok()?;
    let cleaned = clean_comment(raw);
    if cleaned.is_empty() {
        return None;
    }
    let lower = cleaned.to_lowercase();
    // Skip license/copyright banners and shebangs.
    if lower.contains("copyright")
        || lower.contains("license")
        || lower.contains("spdx")
        || cleaned.starts_with("#!")
    {
        return None;
    }
    // First sentence (up to `. `) or whole line, capped at 120 chars.
    let sentence = cleaned
        .split_once(". ")
        .map(|(s, _)| format!("{s}."))
        .unwrap_or(cleaned);
    let mut out = sentence.trim().to_string();
    if out.chars().count() > 120 {
        out = out
            .chars()
            .take(120)
            .collect::<String>()
            .trim_end()
            .to_string();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use carto_model::SymbolKind;

    fn sym<'a>(pf: &'a ParsedFile, name: &str) -> &'a RawSymbol {
        pf.symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("symbol {name} not found"))
    }

    #[test]
    fn detect_lang_extensions() {
        for ext in ["ts", "tsx", "mts", "cts"] {
            assert_eq!(
                detect_lang(&format!("a.{ext}")).as_deref(),
                Some("typescript")
            );
        }
        assert_eq!(detect_lang("a.rs"), None);
        assert_eq!(detect_lang("noext"), None);
    }

    #[test]
    fn unknown_lang_and_bad_input_are_empty() {
        let pf = parse_file("a.rs", "fn main() {}");
        assert!(pf.symbols.is_empty() && pf.refs.is_empty() && pf.lang.is_none());
    }

    #[test]
    fn function_symbol_and_signature() {
        let src = "export function add(a: number, b: number): number {\n  return a + b;\n}\n";
        let pf = parse_file("src/m.ts", src);
        let f = sym(&pf, "add");
        assert_eq!(f.kind, SymbolKind::Function);
        // Signature is the inner declaration header; the `export` modifier sits
        // on the wrapping `export_statement` and is excluded by design
        // (matches the doc examples `class ConfigLoader`, `parse(...)`).
        assert_eq!(
            f.signature.as_deref(),
            Some("function add(a: number, b: number): number")
        );
        assert_eq!(f.stable_key, "src/m.ts#add:function");
        assert_eq!(f.parent_idx, None);
    }

    #[test]
    fn class_method_nesting_and_keys() {
        let src = r#"
export class ConfigLoader {
  parse(src: string): Config {
    return validate(src);
  }
}
"#;
        let pf = parse_file("src/config/load.ts", src);
        let class = sym(&pf, "ConfigLoader");
        assert_eq!(class.kind, SymbolKind::Class);
        assert_eq!(class.signature.as_deref(), Some("class ConfigLoader"));

        let parse = sym(&pf, "parse");
        assert_eq!(parse.kind, SymbolKind::Method);
        // method nests under the class
        let class_idx = pf
            .symbols
            .iter()
            .position(|s| s.name == "ConfigLoader")
            .unwrap();
        assert_eq!(parse.parent_idx, Some(class_idx));
        assert_eq!(
            parse.stable_key,
            "src/config/load.ts#ConfigLoader/parse:method"
        );
        assert_eq!(parse.fqn.as_deref(), Some("ConfigLoader.parse"));
        assert_eq!(
            parse.signature.as_deref(),
            Some("parse(src: string): Config")
        );
    }

    #[test]
    fn interface_type_const_and_arrow() {
        let src = r#"
interface Shape { area: number }
type Id = string | number;
const PI = 3.14;
const mul = (a: number, b: number): number => a * b;
"#;
        let pf = parse_file("src/g.ts", src);
        assert_eq!(sym(&pf, "Shape").kind, SymbolKind::Interface);
        assert_eq!(sym(&pf, "Id").kind, SymbolKind::Type);
        assert_eq!(sym(&pf, "PI").kind, SymbolKind::Const);
        // arrow-bound const is classified as Function
        let mul = sym(&pf, "mul");
        assert_eq!(mul.kind, SymbolKind::Function);
        assert_eq!(
            mul.signature.as_deref(),
            Some("const mul = (a: number, b: number): number =>")
        );
    }

    #[test]
    fn jsdoc_doc_capture() {
        let src = "/** Adds two numbers. */\nexport function add(a: number, b: number): number {\n  return a + b;\n}\n";
        let pf = parse_file("src/m.ts", src);
        assert_eq!(sym(&pf, "add").doc.as_deref(), Some("Adds two numbers."));
    }

    #[test]
    fn line_comment_doc_capture() {
        let src = "// A loader.\nclass Loader {}\n";
        let pf = parse_file("src/m.ts", src);
        assert_eq!(sym(&pf, "Loader").doc.as_deref(), Some("A loader."));
    }

    #[test]
    fn file_purpose_from_header() {
        let src = "/** Loads and validates config.toml. Layered. */\nexport const x = 1;\n";
        let pf = parse_file("src/config/load.ts", src);
        assert_eq!(
            pf.purpose.as_deref(),
            Some("Loads and validates config.toml.")
        );
    }

    #[test]
    fn file_purpose_skips_license() {
        let src = "// Copyright 2026 Acme Inc. All rights reserved.\nexport const x = 1;\n";
        let pf = parse_file("src/x.ts", src);
        assert_eq!(pf.purpose, None);
    }

    #[test]
    fn call_ref_member_last_segment() {
        let src = r#"
export function run(): void {
  this.client.send(payload);
  helper();
}
"#;
        let pf = parse_file("src/m.ts", src);
        let run_idx = pf.symbols.iter().position(|s| s.name == "run").unwrap();
        let calls: Vec<&str> = pf
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Call && r.src_idx == run_idx)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            calls.contains(&"send"),
            "member call last segment: {calls:?}"
        );
        assert!(calls.contains(&"helper"), "plain call: {calls:?}");
    }

    #[test]
    fn import_refs_and_aliases() {
        let src = r#"
import { foo, bar as baz } from "./mod";
import Def from "x";
export function use(): void {
  foo();
}
"#;
        let pf = parse_file("src/m.ts", src);
        let use_idx = pf.symbols.iter().position(|s| s.name == "use").unwrap();
        let imports: Vec<&str> = pf
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Import)
            .map(|r| r.target_name.as_str())
            .collect();
        // named import original name preferred; default import name captured.
        assert!(imports.contains(&"foo"), "imports: {imports:?}");
        assert!(imports.contains(&"bar"), "alias original name: {imports:?}");
        assert!(imports.contains(&"Def"), "default import: {imports:?}");
        // imports here are attributed to the first top-level symbol (`use`).
        assert!(pf
            .refs
            .iter()
            .any(|r| r.kind == EdgeKind::Import && r.src_idx == use_idx));
    }

    #[test]
    fn extends_and_implements_edges() {
        let src = "export class C extends Base implements Iface {}\n";
        let pf = parse_file("src/m.ts", src);
        let c_idx = pf.symbols.iter().position(|s| s.name == "C").unwrap();
        assert!(pf
            .refs
            .iter()
            .any(|r| r.kind == EdgeKind::Inherit && r.target_name == "Base" && r.src_idx == c_idx));
        assert!(pf.refs.iter().any(|r| r.kind == EdgeKind::Implement
            && r.target_name == "Iface"
            && r.src_idx == c_idx));
    }

    #[test]
    fn type_ref_in_param_annotation() {
        let src = r#"
export function load(input: Config): Result {
  return helper(input);
}
"#;
        let pf = parse_file("src/m.ts", src);
        let load_idx = pf.symbols.iter().position(|s| s.name == "load").unwrap();
        let typerefs: Vec<&str> = pf
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef && r.src_idx == load_idx)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            typerefs.contains(&"Config"),
            "param type captured: {typerefs:?}"
        );
    }

    #[test]
    fn tsx_parses_components() {
        let src = "export function App(): JSX.Element {\n  return foo();\n}\n";
        let pf = parse_file("src/App.tsx", src);
        assert_eq!(pf.lang.as_deref(), Some("typescript"));
        assert_eq!(sym(&pf, "App").kind, SymbolKind::Function);
    }
}
