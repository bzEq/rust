//@ run-rustfix
fn main() {
    println!('hello world'); //~ ERROR unterminated character literal
}
