load("@rules_swift//swift:swift.bzl", "swift_test")
load(":layers.bzl", "SIZES", "check_layer")

SWIFT_COPTS = [
    "-swift-version",
    "6",
    "-default-isolation",
    "MainActor",
    "-enable-upcoming-feature",
    "NonisolatedNonsendingByDefault",
    "-enable-upcoming-feature",
    "InferIsolatedConformances",
    "-enable-upcoming-feature",
    "InternalImportsByDefault",
    "-enable-upcoming-feature",
    "MemberImportVisibility",
]

def bilbo_test(name, layer, desktop = False, tags = [], **kwargs):
    check_layer("bilbo_test %s" % name, layer)

    # Screen tests run outside the sandbox to reach the window server, and one at a time.
    screen_tags = ["desktop", "local", "exclusive"] if desktop else []

    swift_test(
        name = name,
        copts = SWIFT_COPTS,
        size = kwargs.pop("size", SIZES[layer]),
        tags = [layer] + screen_tags + tags,
        **kwargs
    )
