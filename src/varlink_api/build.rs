extern crate varlink_generator;

fn main() {
    varlink_generator::cargo_build_tosource("src/varlink_api/org.storage.stratis1.varlink", true);
}
