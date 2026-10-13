"""把合并过的 release PR 从 `autorelease: pending` 翻成 `autorelease: tagged`。

## 为什么需要它

release-please 的发版状态机全在一对标签上：

    release PR 合并 → 打上 `autorelease: pending`
    流程跑完、tag/release 建好 → 翻成 `autorelease: tagged`

只要有**任何一个**合并过的 release PR 还停在 `pending`，
release-please 就会 abort：

    ⚠ There are untagged, merged release PRs outstanding - aborting

于是永远不开新的版本 PR。而本仓库的 `release-please-config.json` 里
`draft: true` + `skip-github-release: true` 这个组合，
release-please **自己不会去建 tag**，所以那个翻转永远不会自己发生。

2026-09-24 起连续 4 次 push main 全部 `publish: skipped`，
根因就是 PR #12 卡在 `pending` 上（连带把 publish.yml 也一起卡死）。
社区同类问题见 googleapis/release-please-action#1205。

## 什么时候跑

合并 `chore(main): release X.Y.Z` 这个 PR **之后**、下一次 push main 之前。
网页上改标签也行，只是多点几下。

## 用法

    python scripts/fix-autorelease-label.py [PR号]

不带参数时自动找出所有还挂着 `autorelease: pending` 的 PR。
改动只有标签，可逆 —— 把标签换回去即可。
"""

import json
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request

REPO = "yanghuiwei/Synapse"
PENDING = "autorelease: pending"
TAGGED = "autorelease: tagged"


def token() -> str:
    """复用 git 凭据里的 token —— 不额外要求配置任何环境变量。"""
    p = subprocess.run(
        ["git", "credential", "fill"],
        input="protocol=https\nhost=github.com\n\n",
        capture_output=True,
        text=True,
    )
    for line in p.stdout.splitlines():
        if line.startswith("password="):
            return line[9:]
    sys.exit("拿不到 GitHub token：git credential fill 没有返回 password")


def call(method: str, path: str, body=None):
    url = f"https://api.github.com/repos/{REPO}{path}"
    headers = {
        "Authorization": f"Bearer {token()}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "reqly-fix-script",
        "Content-Type": "application/json",
    }
    data = json.dumps(body).encode() if body is not None else None
    try:
        r = urllib.request.urlopen(
            urllib.request.Request(url, data=data, method=method, headers=headers),
            timeout=40,
        )
        raw = r.read()
        return r.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:300]


def labels_of(pr_number: int) -> list:
    _, pr = call("GET", f"/pulls/{pr_number}")
    return [x["name"] for x in pr.get("labels", [])] if isinstance(pr, dict) else []


def fix(pr_number: int) -> bool:
    """把一个 PR 的 pending 换成 tagged。返回是否动过手。"""
    _, pr = call("GET", f"/pulls/{pr_number}")
    if not isinstance(pr, dict):
        print(f"  PR#{pr_number} 读不到，跳过")
        return False

    # ⚠️ `tagged` 的语义是「这个版本已经打完 tag 了」。
    # 只对**已合并**的 PR 翻标签 —— 否则等于提前宣布版本已发布，
    # 而它可能最后没合、或合了但构建失败。
    if not pr.get("merged"):
        print(f"  PR#{pr_number} 尚未合并（state={pr.get('state')}），跳过")
        return False

    before = [x["name"] for x in pr.get("labels", [])]
    if PENDING not in before:
        print(f"  PR#{pr_number} 无需处理（labels={before}）")
        return False

    # label 名里有空格，路径段必须百分号编码，否则 GitHub 路由不到
    code, _ = call("DELETE", f"/issues/{pr_number}/labels/{urllib.parse.quote(PENDING)}")
    print(f"  PR#{pr_number} 移除 {PENDING!r} -> HTTP {code}")
    if code not in (200, 204):
        return False

    code, _ = call("POST", f"/issues/{pr_number}/labels", {"labels": [TAGGED]})
    print(f"  PR#{pr_number} 加上 {TAGGED!r} -> HTTP {code}")
    return code in (200, 201)


def main() -> None:
    print(f"仓库 {REPO}")

    if len(sys.argv) > 1:
        targets = [int(sys.argv[1])]
        print(f"指定 PR: {targets}")
    else:
        # 自动挑：已合并、且还挂着 pending 的
        _, issues = call(
            "GET",
            f"/issues?state=closed&labels={urllib.parse.quote(PENDING)}&per_page=50",
        )
        candidates = [i["number"] for i in issues] if isinstance(issues, list) else []
        targets = []
        for n in candidates:
            if call("GET", f"/pulls/{n}")[1].get("merged"):
                targets.append(n)
        print(f"自动找出「已合并且还挂着 {PENDING!r}」的 PR: {targets or '（无）'}")

    if not targets:
        print("没有需要处理的 PR。")
        return

    changed = 0
    for n in targets:
        if fix(n):
            changed += 1

    print()
    for n in targets:
        print(f"  PR#{n} 现在: {labels_of(n)}")
    print(f"\n共改动 {changed} 个 PR。")


if __name__ == "__main__":
    main()