import json,glob,os,sys
for f in sorted(glob.glob(os.path.expanduser("~/.claude/projects/*/*.jsonl"))):
    sid=os.path.basename(f)[:-6]
    ids={json.load(open(g)).get("session_id"):g for g in glob.glob("out-*.json")}
    if sid not in ids: continue
    for line in open(f):
        d=json.loads(line)
        m=d.get("message",{})
        c=m.get("content")
        if isinstance(c,list):
            for b in c:
                if b.get("type")=="tool_result":
                    t=b["content"]
                    if isinstance(t,list): t="".join(x.get("text","") for x in t)
                    lines=t.split("\n")
                    print("==",ids[sid],"chars",len(t),"bytes",len(t.encode()),"lines",len(lines))
                    print("  head:",repr(t[:70])); print("  tail:",repr(t[-250:]))
                    print("  toolUseResult keys:",list(d.get("toolUseResult",{}).keys()) if isinstance(d.get("toolUseResult"),dict) else type(d.get("toolUseResult")))
