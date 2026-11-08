LAYERS = ["unit", "contract", "acceptance", "integration", "snapshot", "e2e", "release"]

SIZES = {
    "unit": "small",
    "contract": "small",
    "acceptance": "small",
    "integration": "medium",
    "snapshot": "small",
    "e2e": "small",
    "release": "large",
}

def check_layer(name, layer):
    if layer not in LAYERS:
        fail("%s: unknown layer '%s', expected one of %s" % (name, layer, LAYERS))
