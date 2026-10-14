#!/usr/bin/env bash
# Get the corpus, the SWE-chat sessions listed at the bottom, untouched, into dataset/corpus-<sha256>.tar.zst, and print
# its path: the archive already there if valid, else the one in dataset.lock, else rebuilt from SWE-chat, which needs
# HF_TOKEN from an account that accepted the dataset's terms.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

DATASET=SALT-NLP/SWE-chat
REVISION=202b071f18e03c79df5a7287565ce2ebc5ee7756
CORPUS_SHA256=f2c9eaac04f9613b816dea3391c566e804faa1252d85ff093ea68a7c17ff0066

out=$root/dataset/corpus-${CORPUS_SHA256:0:12}.tar.zst
cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval/transcripts

die() { echo "fetch-corpus: $*" >&2; exit 1; }
log() { echo "fetch-corpus: $*" >&2; }
sha() { sha256sum "$1" | cut -d' ' -f1; }
# shellcheck source=dataset.sh
source "$root/scripts/dataset.sh"

fetch() {
  if [[ -f $out && $(sha "$out") == "$CORPUS_SHA256" ]]; then
    log "${out#"$root"/} is already valid"
    echo "$out"
    exit 0
  fi
  if fetch_archive "${out##*/}"; then
    [[ $(sha "$out") == "$CORPUS_SHA256" ]] || die "${out##*/} in dataset.lock is not the corpus pinned here"
    echo "$out"
    exit 0
  fi
  [[ -n ${HF_TOKEN:-} ]] || die "HF_TOKEN is not set, and rebuilding the corpus from SWE-chat needs it"

  mkdir -p "$cache" "${out%/*}"
  stage=$(mktemp -d "$cache/../stage.XXXXXX")
  trap 'rm -rf "$stage" "$out.part"' EXIT

  while read -r repo created path; do
    file=$cache/${path##*/}
    if [[ ! -f $file ]]; then
      log "downloading ${path##*/}"
      curl -fsSL --retry 3 -H "Authorization: Bearer $HF_TOKEN" -o "$file.part" \
        "https://huggingface.co/datasets/$DATASET/resolve/$REVISION/$path"
      mv "$file.part" "$file"
    fi
    dest=$stage/corpus/${repo/\//__}/${created//:/}--${path##*/}
    mkdir -p "${dest%/*}"
    ln "$file" "$dest"
  done

  # Fixed order, times and owners, so the same sessions always give the same bytes.
  tar --sort=name --mtime='1970-01-01 00:00:00Z' --owner=0 --group=0 --numeric-owner --format=posix \
    --pax-option='exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime' \
    -C "$stage" -cf - corpus | zstd -q -19 -T0 -o "$out.part"

  got=$(sha "$out.part")
  [[ $got == "$CORPUS_SHA256" ]] || die "archive sha256 is $got, expected $CORPUS_SHA256 (if the sessions are unchanged, clear $cache and run again)"
  mv "$out.part" "$out"
  log "${out#"$root"/} verified"
  echo "$out"
}

# repo, session start, path in the dataset
fetch <<'EOF'
4gray/iptvnator 2026-02-14T23:33:43Z transcripts/745b917c-bc7f-40cf-bc40-f057bae1c579.jsonl
4gray/iptvnator 2026-02-15T22:38:23Z transcripts/cbceb4b9-123e-460a-87dc-81d276f1ffd0.jsonl
4gray/iptvnator 2026-02-21T07:40:56Z transcripts/8016a255-0828-4a01-ae11-a955eebf2fb8.jsonl
4gray/iptvnator 2026-02-21T10:56:27Z transcripts/606f4961-fa58-49f9-b523-a5f0795a4e17.jsonl
4gray/iptvnator 2026-03-22T17:38:17Z transcripts/28b2938e-4a53-4363-908b-127a13b0f672.jsonl
4gray/iptvnator 2026-03-22T17:38:18Z transcripts/72da01ff-064d-40bf-8443-254ceb0d5ab4.jsonl
4gray/iptvnator 2026-03-22T17:38:19Z transcripts/fc5b7c2c-6f83-464d-89f0-ee2ec93be03e.jsonl
4gray/iptvnator 2026-03-25T18:14:55Z transcripts/e340da8c-f607-4672-932d-2d6a9970d20e.jsonl
4gray/iptvnator 2026-03-25T20:05:24Z transcripts/4f2f2cab-a227-46e8-a4c4-d8793ea5a92c.jsonl
4gray/iptvnator 2026-03-25T21:03:22Z transcripts/f58c3ce4-3c8c-4348-82e2-e85a851517fb.jsonl
4gray/iptvnator 2026-03-26T15:02:05Z transcripts/545831c3-f1bb-402d-b3f0-dce48fb6bfb2.jsonl
4gray/iptvnator 2026-03-29T11:36:59Z transcripts/82aee676-d9c1-45ce-8eff-27a36e73980a.jsonl
4gray/iptvnator 2026-03-29T21:51:28Z transcripts/03864394-1085-432c-a261-484b3547126b.jsonl
4gray/iptvnator 2026-03-29T21:51:29Z transcripts/6d8abbfe-ae0d-4e8f-a9e6-be06fad5cd10.jsonl
4gray/iptvnator 2026-04-04T08:09:35Z transcripts/9bdfbec6-977a-410e-a1c3-97af564a811c.jsonl
4gray/iptvnator 2026-04-04T18:19:22Z transcripts/6864ff97-5db6-4c77-9d56-2e75e89488a0.jsonl
4gray/iptvnator 2026-04-11T06:13:46Z transcripts/724f6ca6-33e2-4d82-9eb3-250c77cf26c9.jsonl
4gray/iptvnator 2026-04-11T08:38:59Z transcripts/546d90a2-1403-4daa-9eb6-b46af8983d86.jsonl
4gray/iptvnator 2026-04-12T17:27:58Z transcripts/02eb8642-e65b-4dc2-8f25-0f1de6a24d36.jsonl
4gray/iptvnator 2026-04-16T19:57:25Z transcripts/27761a3e-ccfa-40c2-8ce7-b7fba58263d2.jsonl
MoonshotAI/kimi-cli 2026-02-25T06:42:32Z transcripts/ad4b203d-0231-46e9-ac6c-54f1817cb9dd.jsonl
MoonshotAI/kimi-cli 2026-02-25T06:42:32Z transcripts/d530d2dc-eba4-4a3a-9aa1-0658cddb8af2.jsonl
MoonshotAI/kimi-cli 2026-02-25T08:19:04Z transcripts/629b7931-a460-4113-a0c5-66a7fa4c424e.jsonl
MoonshotAI/kimi-cli 2026-02-25T08:19:05Z transcripts/73d9cbc9-f70b-43d5-a9f2-19184fcfa5df.jsonl
MoonshotAI/kimi-cli 2026-02-25T08:19:07Z transcripts/9205db7e-514f-4fd1-aefe-bc067121b4e8.jsonl
MoonshotAI/kimi-cli 2026-02-25T08:19:10Z transcripts1/d2637ab1-8c63-4089-b382-8195dbd85210.jsonl
MoonshotAI/kimi-cli 2026-02-25T11:22:41Z transcripts1/f14bdf44-1816-4feb-a233-4b501e2e69c6.jsonl
MoonshotAI/kimi-cli 2026-02-28T05:27:32Z transcripts/c7c33335-b4bb-42f9-90a6-9ddaa81066eb.jsonl
MoonshotAI/kimi-cli 2026-02-28T08:35:17Z transcripts/14a06f45-59eb-4cf3-9674-471df5841135.jsonl
MoonshotAI/kimi-cli 2026-02-28T08:35:21Z transcripts/e7d6eabd-0d91-41a1-b2bd-b62f33e94af2.jsonl
MoonshotAI/kimi-cli 2026-02-28T09:38:15Z transcripts1/5da873e1-d952-412e-a236-7a946a9c007e.jsonl
MoonshotAI/kimi-cli 2026-02-28T10:01:05Z transcripts1/3441dc9f-c165-47d9-87ee-89758bdecf8a.jsonl
MoonshotAI/kimi-cli 2026-02-28T10:01:07Z transcripts1/410df8db-cee5-4285-87f9-481ea8ac7263.jsonl
MoonshotAI/kimi-cli 2026-03-06T08:38:06Z transcripts1/4ab5b6c5-4396-4172-9671-a8aa5fb4bb58.jsonl
MoonshotAI/kimi-cli 2026-03-06T08:38:09Z transcripts1/5c0447e5-f29d-434b-bb0b-8898b57a8e78.jsonl
MoonshotAI/kimi-cli 2026-03-06T10:20:35Z transcripts/8f3c3e6d-de5b-41e0-bf00-79caed417af9.jsonl
MoonshotAI/kimi-cli 2026-03-09T12:40:12Z transcripts1/fbddf74d-d590-452d-aa3e-99a79888a119.jsonl
MoonshotAI/kimi-cli 2026-03-09T12:44:16Z transcripts/79ffea47-3da0-4199-9af5-0c28fbe7b372.jsonl
MoonshotAI/kimi-cli 2026-03-10T04:53:18Z transcripts1/516b7270-b0ab-4d34-9e4d-1f05c9ccd20a.jsonl
MoonshotAI/kimi-cli 2026-03-10T07:50:21Z transcripts/7719a9c7-6657-41ab-ad59-9617421811df.jsonl
MoonshotAI/kimi-cli 2026-03-10T09:08:52Z transcripts/116e5aa5-77ff-42f7-b1e4-067c43f61818.jsonl
MoonshotAI/kimi-cli 2026-03-11T07:56:39Z transcripts/7758c4c5-5d29-41e5-bc0a-b3091fd59b0c.jsonl
Safecast/safecast-map-MCP 2026-02-11T13:00:38Z transcripts/b18f7e10-3ef7-4a1a-a610-3c3a57cd3bd2.jsonl
Safecast/safecast-map-MCP 2026-02-16T04:08:01Z transcripts/20f9319f-a0e1-43f7-92da-b744b96c95d9.jsonl
Safecast/safecast-map-MCP 2026-02-16T05:42:01Z transcripts/960e1199-4c77-4e97-9fa1-33fb9498a0a9.jsonl
Safecast/safecast-map-MCP 2026-02-16T07:54:17Z transcripts/b6847497-f5f3-4b52-a8dd-39eafaa75c69.jsonl
Safecast/safecast-map-MCP 2026-02-18T23:36:30Z transcripts/9140e479-d2cf-4f2a-83d2-c43dcac70eee.jsonl
Safecast/safecast-map-MCP 2026-02-20T04:15:08Z transcripts/a851941e-91ed-4706-a4e2-39177a334681.jsonl
Safecast/safecast-map-MCP 2026-02-20T09:19:37Z transcripts/b35d0ffa-3758-4c6a-a9d2-44ffa4ecf1ee.jsonl
Safecast/safecast-map-MCP 2026-02-20T09:32:51Z transcripts/b86e41be-0756-455c-bc32-901d460eef6b.jsonl
Safecast/safecast-map-MCP 2026-02-22T15:24:59Z transcripts/a63effd9-4778-47b8-8b1a-2e5a910fa1af.jsonl
Safecast/safecast-map-MCP 2026-02-23T01:13:28Z transcripts/4349298f-34ca-435c-9350-4d15476e7a4e.jsonl
Safecast/safecast-map-MCP 2026-02-24T07:37:43Z transcripts/8ea091c9-3998-4d23-aa1f-5732adaea61f.jsonl
Safecast/safecast-map-MCP 2026-02-25T09:10:07Z transcripts/d3af4e36-d6e3-4220-9a81-ebca89484b4f.jsonl
Safecast/safecast-map-MCP 2026-02-25T15:10:42Z transcripts/0852daa6-239f-424e-bdd6-79521b1cbc88.jsonl
Safecast/safecast-map-MCP 2026-02-25T22:59:21Z transcripts/323f4137-d5cd-497a-ba7f-d2a1ce9660de.jsonl
Safecast/safecast-map-MCP 2026-02-26T01:47:28Z transcripts/9e2616b7-587c-4f38-be28-d49345b61936.jsonl
Safecast/safecast-map-MCP 2026-02-26T06:19:42Z transcripts/4df2f39a-53b2-4edb-8759-ac9db246bd91.jsonl
Safecast/safecast-map-MCP 2026-02-26T13:09:47Z transcripts/f80e1c19-a344-4583-9cf6-5c9e7daf6737.jsonl
Safecast/safecast-map-MCP 2026-02-27T14:34:17Z transcripts/ba492de5-dae3-4266-bb89-fd5c858092c4.jsonl
Safecast/safecast-map-MCP 2026-02-28T04:57:08Z transcripts/e2172417-c330-4394-80c8-ae75a7da91f5.jsonl
Safecast/safecast-map-MCP 2026-03-02T03:09:53Z transcripts/2da52712-c112-44d9-9bc5-68af6d178f26.jsonl
Safecast/safecast-map-MCP 2026-03-03T13:38:55Z transcripts/bc583e35-b9e8-45bd-bdd1-b7275c4d77d2.jsonl
Safecast/safecast-map-MCP 2026-03-04T01:21:15Z transcripts/38b54a46-5663-47ec-8f87-af920b1f4ba3.jsonl
ababushkin/linear-cycle-drain 2026-05-22T15:04:52Z transcripts/dfb1d8d6-5014-4c8e-8420-98593c05d452.jsonl
ababushkin/linear-cycle-drain 2026-05-23T07:09:27Z transcripts1/df8acbd6-6078-4599-afca-8c1f27104117.jsonl
ababushkin/linear-cycle-drain 2026-05-23T13:41:08Z transcripts1/6f7f96be-151b-447b-9801-d97b0344e7b0.jsonl
ababushkin/linear-cycle-drain 2026-05-23T13:46:07Z transcripts/6437b083-f262-4b53-991a-3b47a8c791d8.jsonl
ababushkin/linear-cycle-drain 2026-05-24T06:11:59Z transcripts1/706c6a41-1528-4a75-a371-dacfb4aa90bb.jsonl
ababushkin/linear-cycle-drain 2026-05-24T06:19:50Z transcripts1/91a4f978-5fbf-40e1-adc9-d0e045f0c932.jsonl
ababushkin/linear-cycle-drain 2026-05-26T10:04:38Z transcripts/497726bc-f43d-45f1-a1ad-5b145ed29226.jsonl
ababushkin/linear-cycle-drain 2026-05-26T10:04:39Z transcripts1/ac6d7e8e-9409-4e35-9a9d-d03b6807b03d.jsonl
ababushkin/linear-cycle-drain 2026-05-26T12:44:54Z transcripts/5b30f67d-eb59-4a1a-9812-1f7b22ed530d.jsonl
ababushkin/linear-cycle-drain 2026-05-28T10:18:10Z transcripts1/e3453eb4-1dd7-4bd0-b6b9-8d80b84b89f1.jsonl
ababushkin/linear-cycle-drain 2026-05-28T10:18:15Z transcripts1/f88a9c6c-fcd4-43da-a7c2-28551f7bb033.jsonl
ababushkin/linear-cycle-drain 2026-06-01T00:23:46Z transcripts/9e1cf68c-904a-4bd4-96b7-0a979cf73c80.jsonl
ababushkin/linear-cycle-drain 2026-06-01T05:52:55Z transcripts/b14da5fd-9704-49b4-9c39-b3daab775550.jsonl
ababushkin/linear-cycle-drain 2026-06-01T05:53:04Z transcripts/c18cf498-dfc9-45cc-a095-8cbd8afeeedb.jsonl
ababushkin/linear-cycle-drain 2026-06-01T06:17:13Z transcripts/222813d0-3fb8-4be4-92b5-8d4af133e06e.jsonl
ababushkin/linear-cycle-drain 2026-06-01T06:31:44Z transcripts1/6a544259-0e69-44d7-a226-521a82b5da75.jsonl
ababushkin/linear-cycle-drain 2026-06-01T06:48:50Z transcripts1/905b4632-e0d8-4446-a47a-05cb1bb26150.jsonl
ababushkin/linear-cycle-drain 2026-06-01T11:33:56Z transcripts1/491e50d7-c5c5-445f-9338-1cfa93277a8a.jsonl
ababushkin/linear-cycle-drain 2026-06-01T11:42:51Z transcripts1/c86f213c-9cb6-4afe-8b13-f4affdef8ebf.jsonl
ababushkin/linear-cycle-drain 2026-06-01T12:15:33Z transcripts/b1f4a288-67fc-47ad-b533-da8896a55254.jsonl
ababushkin/linear-cycle-drain 2026-06-01T12:36:02Z transcripts1/32a22c10-fa23-41f5-a0b9-a991f5a0e9d4.jsonl
ababushkin/linear-cycle-drain 2026-06-09T02:06:08Z transcripts/606a8a53-c1df-4110-a061-50c5b05f0c26.jsonl
ababushkin/linear-cycle-drain 2026-06-10T14:25:17Z transcripts/8bd520d6-f6d8-4720-ab81-ced4d8fc9d8e.jsonl
ababushkin/linear-cycle-drain 2026-06-11T13:14:58Z transcripts1/b03df76c-a39c-480c-91c5-0d7df4fab4d2.jsonl
ababushkin/linear-cycle-drain 2026-06-11T13:50:02Z transcripts/399a9db7-09ca-4a0f-bf6e-dc9b441ed754.jsonl
ababushkin/linear-cycle-drain 2026-06-11T13:50:06Z transcripts1/e55be82e-a00f-4dea-861b-0d6182ca585c.jsonl
ababushkin/linear-cycle-drain 2026-06-16T04:35:34Z transcripts1/155bd31f-b173-435a-9593-143e217cc162.jsonl
ababushkin/linear-cycle-drain 2026-06-16T12:37:14Z transcripts1/0032be59-3fe2-4f0d-a709-ca383d6f1639.jsonl
ababushkin/linear-cycle-drain 2026-06-16T12:37:15Z transcripts/0ea14c36-4388-4f5f-9e9c-7327e3607066.jsonl
ababushkin/linear-cycle-drain 2026-06-17T02:49:20Z transcripts1/0567308d-82ec-40e4-871a-f12a9ac1109b.jsonl
ababushkin/linear-cycle-drain 2026-06-18T02:43:13Z transcripts1/aa44615b-0574-43ae-ba73-c375f172efe7.jsonl
ababushkin/linear-cycle-drain 2026-06-18T12:34:58Z transcripts/6d19145b-86d2-4b0d-9077-624706537065.jsonl
ababushkin/linear-cycle-drain 2026-06-19T20:42:30Z transcripts1/f9aa2d25-b5d5-4e19-942f-cb160681c3ec.jsonl
ababushkin/linear-cycle-drain 2026-06-22T05:32:03Z transcripts1/311642ee-507c-40ee-977b-417565577d69.jsonl
ababushkin/linear-cycle-drain 2026-06-22T05:32:03Z transcripts/cae6a62c-fe54-4356-a6b6-817832cc5da9.jsonl
ababushkin/linear-cycle-drain 2026-06-23T06:16:00Z transcripts1/8d6ea8ad-45c7-488b-b1f9-e9583c1d6023.jsonl
ababushkin/linear-cycle-drain 2026-06-23T06:22:30Z transcripts1/2b015bc9-f784-44bc-a1f6-65c1d66f41ec.jsonl
ababushkin/linear-cycle-drain 2026-06-23T10:34:19Z transcripts1/fc19ab21-914f-47c3-b619-c21eaf38561f.jsonl
ababushkin/linear-cycle-drain 2026-06-23T11:03:38Z transcripts/f9197be4-db62-490d-8eef-157574f44b34.jsonl
achildrenmile/bosarsalog 2026-02-15T10:10:01Z transcripts1/4fad57e0-08c3-465e-b5c7-b843989b5a46.jsonl
achildrenmile/bosarsalog 2026-02-15T11:56:25Z transcripts/2a33c1f0-1609-4959-9904-91ae778bddda.jsonl
achildrenmile/bosarsalog 2026-02-16T07:15:37Z transcripts1/461ccc04-e951-4f15-b21f-f1c732dd3667.jsonl
achildrenmile/bosarsalog 2026-02-16T13:43:20Z transcripts1/e0ab8722-350a-4e36-83d7-22c437ec33cd.jsonl
achildrenmile/bosarsalog 2026-02-16T14:43:14Z transcripts1/dc8a8949-5565-4a85-a67a-3cadf10e1936.jsonl
achildrenmile/bosarsalog 2026-02-16T18:14:52Z transcripts1/6a216e27-0036-4285-9be7-fe9753b90aca.jsonl
achildrenmile/bosarsalog 2026-02-16T19:31:54Z transcripts1/f3619e03-9d1e-4cfc-95cc-2af785639c53.jsonl
achildrenmile/bosarsalog 2026-02-17T16:11:36Z transcripts1/fea1333a-a7e9-44e5-9a3d-9e6e2cf0c4e3.jsonl
achildrenmile/bosarsalog 2026-02-17T18:29:05Z transcripts1/33d8e7b4-37c8-4a39-9bf1-1d2b56aa8be3.jsonl
achildrenmile/bosarsalog 2026-02-24T19:14:35Z transcripts1/7d36dfe9-1644-4da7-aeb9-ef74a092d12c.jsonl
achildrenmile/bosarsalog 2026-02-24T19:35:53Z transcripts1/6a97a4cc-3b28-4ddc-90a0-a07287463f2a.jsonl
achildrenmile/bosarsalog 2026-02-24T20:13:30Z transcripts/47641f8a-2768-48b1-80d1-f5ce1ff72e81.jsonl
achildrenmile/bosarsalog 2026-02-24T20:13:31Z transcripts1/8fe1ab72-5959-4535-87e9-f9e120378f3d.jsonl
achildrenmile/bosarsalog 2026-02-25T06:25:56Z transcripts/013e9a46-7047-41cf-9e91-46c53d5aca1f.jsonl
achildrenmile/bosarsalog 2026-02-25T06:50:38Z transcripts1/96d8ab46-33fc-42f9-b250-da5ea8eafd3a.jsonl
achildrenmile/bosarsalog 2026-02-25T07:36:26Z transcripts/d9f35ffc-1637-4992-a696-d51accfa31c2.jsonl
achildrenmile/bosarsalog 2026-02-25T08:25:55Z transcripts/d40b7114-7701-4b28-b9dd-b300e7aa9394.jsonl
achildrenmile/bosarsalog 2026-03-02T09:30:17Z transcripts1/9ff48ab2-2ddc-4f81-a10f-8f3836db2506.jsonl
achildrenmile/bosarsalog 2026-03-02T19:28:35Z transcripts/abcc5a4f-9583-427f-85cf-1c4283b3d576.jsonl
achildrenmile/bosarsalog 2026-03-02T22:46:03Z transcripts1/ccf13571-f85b-4be8-b142-9e5e060af664.jsonl
achildrenmile/bosarsalog 2026-03-03T07:21:54Z transcripts1/613e732b-201d-4dba-84de-089659d98cfc.jsonl
achildrenmile/bosarsalog 2026-03-15T21:15:11Z transcripts1/1f303b0c-b261-4c38-b120-381193d1d1dd.jsonl
achildrenmile/bosarsalog 2026-05-10T18:50:52Z transcripts1/6a11b066-ab08-47ec-837f-bea27bda801e.jsonl
achildrenmile/bosarsalog 2026-06-01T05:10:44Z transcripts/cd0d2355-a2a4-4701-a7ca-c25458bfafef.jsonl
entireio/go-nuts 2026-07-10T11:51:43Z transcripts/b19adb79-1a3a-463d-b5df-57c95724b4b7.jsonl
entireio/go-nuts 2026-07-10T13:00:25Z transcripts/80c151b5-6e53-4d70-a24f-e06b58b2d83d.jsonl
entireio/go-nuts 2026-07-14T08:27:03Z transcripts1/72bff985-4a1a-4a24-ab7e-a7ad3dba9c7e.jsonl
entireio/go-nuts 2026-07-16T11:12:05Z transcripts/033c3992-c4df-4e86-a73c-f74bbec4bd33.jsonl
entireio/go-nuts 2026-07-17T10:56:38Z transcripts/9a9a1b8a-7091-45cd-961d-b5a3f1341316.jsonl
entireio/go-nuts 2026-08-10T18:01:28Z transcripts1/543cc1b7-5c73-41d8-a92b-d8e90b5c5b50.jsonl
entireio/go-nuts 2026-08-12T14:07:59Z transcripts1/92803846-44fa-4a98-9cfa-8f103b196f32.jsonl
entireio/go-nuts 2026-08-12T16:33:19Z transcripts/fdaa4d69-95e1-4f90-a1a4-c28bb1a1c3ac.jsonl
entireio/go-nuts 2026-08-13T07:56:01Z transcripts1/7613ebe4-ca24-471a-8d20-f6bc356c5f8e.jsonl
entireio/go-nuts 2026-08-13T12:09:10Z transcripts/86391b05-8edc-4261-97ce-c28aa0be977f.jsonl
entireio/go-nuts 2026-08-14T14:39:32Z transcripts/d2e2972a-5403-4def-9c9c-8f3771242207.jsonl
entireio/go-nuts 2026-08-14T15:40:48Z transcripts1/eb0b9643-97cb-4ad8-a938-caaf0606b585.jsonl
henryph24/neuralips26 2026-04-07T17:10:10Z transcripts/1afea02a-acd8-4499-9d25-279b3a0c11a9.jsonl
henryph24/neuralips26 2026-04-08T18:29:29Z transcripts/db4a8d3b-caab-4f7d-92a6-ade807e32f39.jsonl
henryph24/neuralips26 2026-04-09T09:02:11Z transcripts/582b8944-aace-48ed-853a-155dd9b5ff68.jsonl
henryph24/neuralips26 2026-04-09T13:04:00Z transcripts/9fde1548-1a2b-4dc8-b9fa-6150f8d3d21d.jsonl
henryph24/neuralips26 2026-04-09T16:44:26Z transcripts/5ab2777b-c8a9-4ab2-8d84-364c71597977.jsonl
henryph24/neuralips26 2026-04-09T16:44:28Z transcripts/b2b5366b-6963-4be6-be4a-ce094982febd.jsonl
henryph24/neuralips26 2026-04-10T07:54:52Z transcripts/fc463366-d96e-469b-b8ca-4c0c4afa40e2.jsonl
henryph24/neuralips26 2026-04-10T10:26:13Z transcripts/4df34b53-9a00-4833-b743-861a1f104931.jsonl
henryph24/neuralips26 2026-04-14T05:07:15Z transcripts/01bdfe47-aa15-42ed-bb6a-71aedb7e8063.jsonl
henryph24/neuralips26 2026-04-14T12:18:02Z transcripts/19e7d2d8-dbb5-461b-acd5-341bf783f757.jsonl
henryph24/neuralips26 2026-04-15T15:09:11Z transcripts/370a641a-4f3f-4f1b-9338-a4bdaf027e61.jsonl
henryph24/neuralips26 2026-04-15T15:09:18Z transcripts/a34b6199-9b61-4012-9e65-841a686ad2b1.jsonl
henryph24/neuralips26 2026-04-17T08:54:48Z transcripts/c5a9377e-2658-48ae-871b-1af5067c1578.jsonl
henryph24/neuralips26 2026-04-18T03:05:27Z transcripts/acb3b7d2-0f13-4bfe-bfc9-166e59c3c607.jsonl
henryph24/neuralips26 2026-04-18T10:26:19Z transcripts/dd693f48-c8c7-464a-9721-c339ef9012ee.jsonl
henryph24/neuralips26 2026-04-20T03:51:15Z transcripts1/1439812d-4c89-4bd0-ad09-396f408f413b.jsonl
henryph24/neuralips26 2026-04-20T11:09:29Z transcripts/7e95990f-b0aa-4d9a-a78c-0389c772eba7.jsonl
henryph24/neuralips26 2026-04-20T11:09:32Z transcripts/950afe9d-51e4-4401-8ff6-3f00569d50c3.jsonl
henryph24/neuralips26 2026-04-21T10:10:44Z transcripts1/73aa526d-b2de-42af-9096-dd737fc6c36e.jsonl
henryph24/neuralips26 2026-04-22T07:23:15Z transcripts1/4a095dfb-66ac-41d7-90e0-2f69f6bcc013.jsonl
henryph24/neuralips26 2026-04-22T07:23:18Z transcripts/8ef779a5-0625-43ff-a62e-44cdbbd544e8.jsonl
henryph24/neuralips26 2026-04-24T01:15:46Z transcripts1/281fca7c-6430-4ae0-ba1f-02aa219048d6.jsonl
ngngardner/readaloud 2026-03-12T23:21:53Z transcripts/ca2f8b89-21d0-4da2-91f1-7b17c04613a4.jsonl
ngngardner/readaloud 2026-05-01T20:58:34Z transcripts1/313a90af-7255-4653-b5ba-0440308fb9fa.jsonl
ngngardner/readaloud 2026-05-01T20:58:34Z transcripts1/c28fe12d-c40c-4c90-961f-12d3f036eeb7.jsonl
ngngardner/readaloud 2026-05-01T21:28:24Z transcripts1/45a8de26-c39a-49c3-8b76-9f9becda2320.jsonl
ngngardner/readaloud 2026-05-02T16:24:41Z transcripts/23193cc2-0713-490e-8c51-0225ef558d27.jsonl
ngngardner/readaloud 2026-05-02T17:42:40Z transcripts1/e1d38ef3-2ac1-4bd6-9223-272f356a7223.jsonl
ngngardner/readaloud 2026-05-02T18:13:09Z transcripts/72650cc7-9470-4d11-bc47-98048e548bb6.jsonl
ngngardner/readaloud 2026-05-02T19:30:09Z transcripts1/48cabd63-40e3-43d4-bebc-9c9a6dcba00f.jsonl
ngngardner/readaloud 2026-05-03T04:22:12Z transcripts/262480f3-2ca6-4d1c-a220-6a8fb0144733.jsonl
ngngardner/readaloud 2026-05-03T16:12:53Z transcripts/5c8b91dd-a09a-495c-8f3c-4fb36e3a03c8.jsonl
ngngardner/readaloud 2026-05-03T16:16:50Z transcripts1/a2addf84-7536-43ce-b61c-06e20fa3b41a.jsonl
ngngardner/readaloud 2026-05-03T18:15:06Z transcripts1/9606a385-7b12-436e-b894-ad06cab661a8.jsonl
ngngardner/readaloud 2026-05-05T21:28:33Z transcripts/c098d994-4b88-45a9-979b-ef3874e4e481.jsonl
ngngardner/readaloud 2026-05-06T14:22:20Z transcripts1/ba734d0c-9026-4831-975d-19a5647f2c1f.jsonl
ngngardner/readaloud 2026-05-06T17:57:39Z transcripts/c6378f6c-371c-4701-8e53-fb76bad96973.jsonl
ngngardner/readaloud 2026-05-06T17:58:37Z transcripts1/853aaf30-d524-40e6-a8d5-3f11a23528b3.jsonl
ngngardner/readaloud 2026-05-06T20:31:16Z transcripts/990360d7-95d9-487d-8f2e-53193a3729fd.jsonl
ngngardner/readaloud 2026-05-06T20:54:35Z transcripts/08b585ee-be39-4357-841f-95eee8b7d50c.jsonl
ngngardner/readaloud 2026-05-07T01:24:55Z transcripts1/34c5688c-0333-41ff-b48b-e7ede2cff8e5.jsonl
ngngardner/readaloud 2026-05-07T04:19:23Z transcripts1/13449216-cf75-4462-8d83-7be1866b7fa4.jsonl
ngngardner/readaloud 2026-05-07T20:26:19Z transcripts/bd624c69-1d74-429c-af78-aee1df15b2a4.jsonl
ngngardner/readaloud 2026-05-08T14:10:09Z transcripts1/5ed9247e-3e8d-40e0-b7da-7315299f837c.jsonl
ngngardner/readaloud 2026-05-08T16:27:04Z transcripts/12b80441-7933-4266-8ec0-f0cb5ee1aa8d.jsonl
ngngardner/readaloud 2026-05-08T16:57:46Z transcripts1/0bc4f79f-7baa-4b5e-b4d9-09d578b58487.jsonl
ngngardner/readaloud 2026-05-25T01:29:24Z transcripts/fa395bcc-8a81-4a81-ac67-432b5982fbd4.jsonl
ngngardner/readaloud 2026-06-10T05:34:08Z transcripts/2bbaf4f1-2ec1-434b-85e9-5bd06c393481.jsonl
ngngardner/readaloud 2026-06-10T14:37:21Z transcripts1/0157b421-626f-47d1-9caf-6cea94c6d82f.jsonl
ngngardner/readaloud 2026-06-10T17:08:48Z transcripts1/514f98a6-2f92-4f7f-9650-58efbb1e8439.jsonl
ngngardner/readaloud 2026-06-11T06:14:49Z transcripts1/72187cdc-53e9-4b07-8a24-5499cc3b615b.jsonl
ngngardner/readaloud 2026-06-11T22:06:50Z transcripts/89c980cb-26d4-445b-82ba-1f08d74a3d41.jsonl
ngngardner/readaloud 2026-08-13T00:28:27Z transcripts/d85efab2-d937-4964-89ba-f1c768b6a448.jsonl
ngngardner/readaloud 2026-08-16T04:20:15Z transcripts1/c108bb48-f63b-4f4d-b601-76bd6a8a03c9.jsonl
ngngardner/readaloud 2026-08-19T02:12:05Z transcripts/087e6694-3a24-4799-abbc-be0cad08e97f.jsonl
ngngardner/readaloud 2026-08-19T03:14:05Z transcripts1/5816f6dd-7d64-4034-8949-b6320deddeb7.jsonl
port-experimental/port-cli 2026-03-24T16:55:28Z transcripts1/8a6e7859-a9d6-4d6a-ba12-5cb86da805ec.jsonl
port-experimental/port-cli 2026-03-24T17:02:40Z transcripts1/d0dbaa44-f91f-4495-b738-5e12e8baef63.jsonl
port-experimental/port-cli 2026-04-07T15:59:40Z transcripts1/81d8640e-ca5d-4e57-b521-788834f2da1f.jsonl
port-experimental/port-cli 2026-04-10T11:22:04Z transcripts1/48ec495f-f3f7-4673-baac-d5459b668397.jsonl
port-experimental/port-cli 2026-04-10T11:27:01Z transcripts/1e80793e-6a6b-4d63-a3c3-922f63cdb4ca.jsonl
port-experimental/port-cli 2026-04-10T15:19:37Z transcripts/271d2f76-32c7-4cc5-84e5-3a8f03b6a4c2.jsonl
port-experimental/port-cli 2026-04-14T15:37:34Z transcripts/32195508-517a-4ee5-89bb-da4ac238a2e3.jsonl
port-experimental/port-cli 2026-04-21T08:22:08Z transcripts/8ff2c48c-0776-48a4-a4e4-316ee5b03709.jsonl
port-experimental/port-cli 2026-04-21T12:19:49Z transcripts1/51813c45-a679-41be-ad38-f7bb638e15be.jsonl
port-experimental/port-cli 2026-05-07T12:52:31Z transcripts1/43b7432f-ef8d-4b1e-b03a-4bca47732f76.jsonl
port-experimental/port-cli 2026-05-07T12:52:32Z transcripts/a79e3086-7009-4b29-953e-e80bd5a09219.jsonl
port-experimental/port-cli 2026-05-07T12:53:01Z transcripts1/57261a82-276b-476b-b85c-d4b7ec69bedd.jsonl
port-experimental/port-cli 2026-05-08T16:01:13Z transcripts1/d173ba9b-4559-43d8-b674-4b953e172b3e.jsonl
port-experimental/port-cli 2026-05-18T11:58:01Z transcripts1/3beb8865-59c1-4916-aa29-964f999d88ff.jsonl
port-experimental/port-cli 2026-05-25T08:37:16Z transcripts1/83808ed3-13fe-409d-93ca-c1a6f4d66bc7.jsonl
port-experimental/port-cli 2026-05-28T14:26:56Z transcripts/7f10362b-2519-4917-9385-39d52baeb667.jsonl
port-experimental/port-cli 2026-05-28T14:27:39Z transcripts1/e9e702ed-d76e-4066-bedb-216798151f04.jsonl
port-experimental/port-cli 2026-05-29T09:34:13Z transcripts1/f7c1ac0d-c9a7-4b49-8d53-8fe0faaa28a3.jsonl
port-experimental/port-cli 2026-06-05T16:08:13Z transcripts1/a908bfd9-5354-444d-8346-984322b716bd.jsonl
port-experimental/port-cli 2026-06-17T07:55:20Z transcripts/9b9c9a6e-1f75-442c-92c5-1687ad2f94f5.jsonl
port-experimental/port-cli 2026-06-17T07:56:11Z transcripts1/42df428e-db40-4575-a66a-ab401a72519a.jsonl
port-experimental/port-cli 2026-06-17T14:08:37Z transcripts1/200a688b-179b-47f1-ba59-b7c7df76f028.jsonl
port-experimental/port-cli 2026-06-17T14:09:44Z transcripts/1ae52fa7-2bc6-4e03-a293-c9288649e228.jsonl
port-experimental/port-cli 2026-06-17T20:54:06Z transcripts1/573edfd7-da00-4572-9e05-ede8c1272085.jsonl
port-experimental/port-cli 2026-06-18T20:12:04Z transcripts1/14ddf933-81b5-40dc-a30e-59748d147cb0.jsonl
port-experimental/port-cli 2026-06-18T20:53:27Z transcripts1/bf44fc35-39e4-4be9-9eaa-a2275780cdcd.jsonl
port-experimental/port-cli 2026-06-24T10:41:51Z transcripts/b4014210-53b5-4f24-8b73-01a80fdb3257.jsonl
port-experimental/port-cli 2026-06-25T10:25:41Z transcripts1/78c61f4b-fac0-4488-afa1-d63040218ae8.jsonl
port-experimental/port-cli 2026-07-02T10:40:08Z transcripts/144bb8bd-3844-473f-9808-ba96d5ef4026.jsonl
port-experimental/port-cli 2026-07-06T17:05:20Z transcripts1/1db8024f-af27-4682-a497-50e95a046f1b.jsonl
scottdensmore/contoso 2026-08-12T03:58:43Z transcripts1/4368fb93-c8cc-4d91-99ab-3d6d4964d7d7.jsonl
scottdensmore/contoso 2026-08-16T01:03:54Z transcripts/6981b884-fc97-4527-9bb8-dfe067a590b0.jsonl
scottdensmore/contoso 2026-08-17T06:03:46Z transcripts1/7e777541-03c5-42ee-bd9f-59ae0f93daad.jsonl
scottdensmore/contoso 2026-08-17T06:10:42Z transcripts1/32326432-9961-4c06-ab5c-44ff606dccde.jsonl
scottdensmore/contoso 2026-08-17T17:04:55Z transcripts1/7d209c9f-4ef0-437a-9a43-a48e7ac452d3.jsonl
scottdensmore/contoso 2026-08-17T23:40:04Z transcripts/89a8e71c-0cc2-4723-bb0b-893448362963.jsonl
scottdensmore/contoso 2026-08-18T18:48:09Z transcripts/32aea79c-3855-4190-9ae4-5916c6d50c99.jsonl
scottdensmore/contoso 2026-08-19T21:37:50Z transcripts1/93f6c8cf-8698-4af0-962b-05046cda36d1.jsonl
scottdensmore/contoso 2026-08-21T04:53:55Z transcripts1/76d36c20-8877-449b-bb72-3ba33106b3cc.jsonl
EOF
