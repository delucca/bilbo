load("@rules_rust//rust:defs.bzl", "rust_test")
load(":layers.bzl", "SIZES", "check_layer")

def bilbo_rust_test(name, layer, tags = [], **kwargs):
    check_layer("bilbo_rust_test %s" % name, layer)
    rust_test(
        name = name,
        size = kwargs.pop("size", SIZES[layer]),
        tags = [layer] + tags,
        **kwargs
    )
