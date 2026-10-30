load("@rules_swift//swift:swift.bzl", "swift_test")

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

LAYERS = ["unit", "contract", "acceptance", "integration", "snapshot", "e2e", "release"]

_SIZES = {
    "unit": "small",
    "contract": "small",
    "acceptance": "small",
    "integration": "medium",
    "snapshot": "small",
    "e2e": "small",
    "release": "large",
}

def bilbo_test(name, layer, desktop = False, tags = [], **kwargs):
    if layer not in LAYERS:
        fail("bilbo_test %s: unknown layer '%s', expected one of %s" % (name, layer, LAYERS))

    # Screen tests run outside the sandbox to reach the window server, and one at a time.
    screen_tags = ["desktop", "local", "exclusive"] if desktop else []

    swift_test(
        name = name,
        copts = SWIFT_COPTS,
        size = kwargs.pop("size", _SIZES[layer]),
        tags = [layer] + screen_tags + tags,
        **kwargs
    )
