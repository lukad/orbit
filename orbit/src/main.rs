use orbit_common::SourceId;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("Expected a path argument");
    let source = std::fs::read_to_string(&path).expect("Failed to read file");
    let source_id = SourceId::new(0);
    let tokens = orbit_parser::lexer::lex(source_id, &source).unwrap();
    let ast = orbit_parser::parser::parse_chunk(source_id, &tokens).unwrap();
    println!("{:#?}", ast);
}
