use bilbo_layout::Tree;

#[test]
#[should_panic(expected = "cannot read no-such-folder/src:")]
fn a_missing_folder_stops_the_scan() {
    Tree::load("no-such-folder");
}
