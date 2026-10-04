import sys
# usage: gen.py bytes <KB> | gen.py lines <N>
mode, n = sys.argv[1], int(sys.argv[2])
label = f"{mode}-{n}"
out = []
total = 0
if mode in ("bytes","exact"):
    target = n * 1024 if mode=="bytes" else n
    i = 1
    while True:
        line = f"{i}\t" + ("word " * 14).rstrip() + f" {i:06d}"
        if total + len(line) + 1 > target:
            break
        out.append(line); total += len(line) + 1; i += 1
else:
    for i in range(1, n + 1):
        out.append(f"{i}\tshort line {i}")
out.append(f"-- end probe {label} --")
sys.stdout.write("\n".join(out) + "\n")
