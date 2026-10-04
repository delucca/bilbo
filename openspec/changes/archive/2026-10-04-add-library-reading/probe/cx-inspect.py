import json,glob,re,sys
for f in sorted(glob.glob("cx/log-*.jsonl"), key=lambda x:(x.split('-')[1], int(re.findall(r'\d+',x)[0]))):
    reqs=[json.loads(l) for l in open(f)]
    for it in reqs[-1]["body"]["input"]:
        if it.get("type")=="function_call_output":
            o=it["output"]
            t=o if isinstance(o,str) else "".join(x.get("text","") for x in o)
            nums=[int(m) for m in re.findall(r'^(\d+)\t',t,re.M)]
            gaps=[(a,b) for a,b in zip(nums,nums[1:]) if b!=a+1]
            print("==",f,"chars",len(t),"bytes",len(t.encode()),"lines",t.count("\n")+1,"firstline",nums[:1],"lastnum",nums[-1:],"gaps",gaps,"endmarker","-- end probe" in t)
            if len(sys.argv)>1:
                print(repr(t[:300])); 
            for m in re.finditer(r'[^\n]*(truncat|Total output|omitted|tokens)[^\n]*',t): print("   notice:",repr(m.group(0)[:200]))
