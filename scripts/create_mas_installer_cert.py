#!/usr/bin/env python3
"""通过 App Store Connect API 签发 Mac App Store 用的 installer 证书。

## 为什么需要它

上传 Mac App Store 包时，`productbuild` 产出的那个 pkg 本身要用
**"3rd Party Mac Developer Installer"** 证书签名，否则 ASC 拒收：

    90237 The product archive package's signature is invalid.
          Ensure that it is signed with your
          "3rd Party Mac Developer Installer" certificate.

（实测 2026-10-01 首次真实上传时踩到。）

钥匙串里常有的 `Developer ID Installer` **不能替代**它 —— 那是 Developer ID
分发用的证书，ASC 不认。要拿到 App Store 版的，只能让 Apple 签发，而
Apple 只允许通过 API 创建、不允许通过 API 导出私钥。所以流程是：

1. 本地生成私钥 + CSR
2. 通过 API 请 Apple 签发（CSR 送上去，私钥留下）
3. 拿回证书，导成 p12 备用，同时导进钥匙串

## 幂等

同类型证书已存在就直接复用 —— ASC 不允许同一类型有多张 active 证书，
重复申请会得到 `ENTITY_ERROR`。所以这个脚本**不会**重复签发。

## 用法

```bash
# 只看现状
python3 scripts/create_mas_installer_cert.py --show

# 需要时签发并导入钥匙串
python3 scripts/create_mas_installer_cert.py
```
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "mas_profile", ROOT / "scripts/create_mas_profile.py"
)
mas = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(mas)

STORE = Path.home() / ".config/mddock"
KEY = STORE / "MacSlim_MAS_installer_key.pem"
CSR = STORE / "MacSlim_MAS_installer.csr"
CERT = STORE / "MacSlim_MAS_installer.crt.pem"
P12 = STORE / "MacSlim_MAS_installer.p12"
COMMON_NAME = "3rd Party Mac Developer Installer: Beijing VGO Co;Ltd (5XNDF727Y6)"

CERT_TYPE = "MAC_INSTALLER_DISTRIBUTION"


def info(message: str) -> None:
    print(f"  {message}")


def fail(message: str) -> None:
    print(f"错误: {message}")
    raise SystemExit(1)


def find_existing(env: dict) -> dict | None:
    for item in mas.get_all("/certificates", env):
        if item.get("attributes", {}).get("certificateType") == CERT_TYPE:
            return item
    return None


def generate_key_and_csr() -> tuple[bytes, str]:
    STORE.mkdir(parents=True, exist_ok=True)
    if not KEY.exists():
        subprocess.run(
            [
                "openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes",
                "-keyout", str(KEY), "-out", str(CSR),
                "-subj", "/CN=Mac Installer/O=Beijing VGO Co;Ltd/C=CN",
            ],
            check=True,
            capture_output=True,
        )
        info(f"已生成私钥与 CSR：{KEY.name}、{CSR.name}")
    return CSR.read_bytes(), CSR.read_text(encoding="utf-8")


def request_certificate(env: dict) -> dict:
    csr_bytes, csr_text = generate_key_and_csr()
    created = mas.request(
        "POST",
        "/certificates",
        env,
        {
            "data": {
                "type": "certificates",
                "attributes": {
                    "certificateType": CERT_TYPE,
                    "csrContent": csr_text,
                },
            }
        },
    )
    info(f"已向 Apple 申请签发：{created['data']['id']}")
    return created["data"]


def fetch_certificate_content(env: dict, certificate_id: str) -> str | None:
    """轮询拿证书内容（签发是异步的）。"""
    import time

    for _ in range(15):
        fetched = mas.request("GET", f"/certificates/{certificate_id}", env)
        content = fetched["data"].get("attributes", {}).get("certificateContent")
        if content:
            return content
        time.sleep(2)
    return None


def show(env: dict) -> None:
    existing = find_existing(env)
    if not existing:
        print("钥匙串与 ASC 上都没有 MAC_INSTALLER_DISTRIBUTION 证书。")
        print("跑 `python3 scripts/create_mas_installer_cert.py` 签发。")
        return
    info(f"ASC 上已有：{existing['id']}")
    info(f"displayName: {existing['attributes'].get('displayName')}")
    info(f"证书类型: {existing['attributes'].get('certificateType')}")


def export_p12() -> None:
    """把证书 + 私钥打成 p12 备用。

    私钥永远留在本机 —— Apple 只拿到 CSR，从不持有私钥。所以这份 p12
    就是**唯一**的备份，弄丢了只能重新走一遍申请。
    """
    if P12.exists():
        return
    password = subprocess.run(
        ["openssl", "rand", "-base64", "24"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    subprocess.run(
        [
            "openssl", "pkcs12", "-export", "-out", str(P12),
            "-inkey", str(KEY), "-in", str(CERT),
            "-passout", f"pass:{password}",
        ],
        check=True,
        capture_output=True,
    )
    (STORE / "MacSlim_MAS_installer_p12_password.txt").write_text(
        password + "\n", encoding="utf-8"
    )
    os_chmod(P12)
    info(f"已导出 p12（口令另存于 {P12.stem}_password.txt）")


def import_to_keychain() -> None:
    """导入 p12 到登录钥匙串，并让 codesign / productsign 能非交互使用。

    三个坑，都在注释里：
    - 必须导 **p12**：只导 .crt 的话钥匙串里没有私钥，
      `find-identity` 里根本不出现这个身份，而日志会显示「导入成功」。
    - 不要传 `-k`：那个参数是「打开这个 keychain 文件」，传进来会被当成
      那样解释，于是报 "The specified keychain is not a valid keychain file"。
    - 用 `-A` 而不是 `-T`：`-T` 只把列出的程序加进 ACL，其余程序访问私钥
      仍会弹确认框；在没有 GUI 的环境里那个框弹不出来，调用方就**挂住**
      （实测 productsign 会静默卡住，被外部超时掐断后留下一个所有条目
      都是 0 字节的半成品包，签名状态 invalid）。
    """
    password = (STORE / "MacSlim_MAS_installer_p12_password.txt").read_text().strip()
    imported = subprocess.run(
        ["security", "import", str(P12), "-P", password, "-A"],
        capture_output=True,
        text=True,
    )
    if imported.returncode != 0:
        info(f"导入钥匙串失败（若已存在可忽略）：{imported.stderr.strip()[:120]}")
    else:
        info("已导入钥匙链")
    identities = subprocess.run(
        ["security", "find-identity", "-v"], capture_output=True, text=True
    ).stdout
    hit = [
        line.strip()
        for line in identities.splitlines()
        if "3rd Party Mac Developer Installer" in line
    ]
    if hit:
        info("可用身份：" + hit[0])
    else:
        fail("证书导入后仍找不到对应身份，检查钥匙串")


def ensure(env: dict) -> None:
    existing = find_existing(env)
    if existing:
        existing_id = existing["id"]
        info(f"复用现有证书 {existing_id}")
    else:
        created = request_certificate(env)
        existing_id = created["id"]
        info(f"证书已保存：{CERT}")

    raw = CERT.read_bytes() if CERT.exists() else b""
    if not pem_is_wellformed(raw):
        info("现有证书文件结构不可用（或还没有），从 ASC 重新取一份")
        content = fetch_certificate_content(env, existing_id)
        if not content:
            fail("从 ASC 取不到证书内容")
        CERT.write_bytes(der_to_pem(_b64(content)))
    else:
        info(f"证书文件已就绪：{CERT}")

    export_p12()
    import_to_keychain()


def os_chmod(path: Path) -> None:
    path.chmod(0o600)


def _b64(text: str) -> bytes:
    import base64

    return base64.b64decode(text)


def _strip_pem(raw: bytes) -> bytes:
    """剥掉 PEM 头尾，只留 base64 正文（无论折行是否正确）。"""
    lines = raw.split(b"\n")
    body = [
        line.strip()
        for line in lines
        if line.strip() and not line.strip().startswith(b"-----")
    ]
    return b"".join(body)


def pem_is_wellformed(raw: bytes) -> bool:
    """结构校验：头尾各一行，正文每行 64 字符（末行可短），且能解出 DER。"""
    import base64

    lines = [line.strip() for line in raw.strip().split(b"\n")]
    if len(lines) < 3:
        return False
    if lines[0] != b"-----BEGIN CERTIFICATE-----":
        return False
    if lines[-1] != b"-----END CERTIFICATE-----":
        return False
    body = lines[1:-1]
    if any(len(line) != 64 for line in body[:-1]):
        return False
    if not body or len(body[-1]) > 64:
        return False
    try:
        base64.b64decode(b"".join(body), validate=True)
    except Exception:
        return False
    return True


def der_to_pem(der: bytes) -> bytes:
    """DER → PEM。

    Apple 的 certificates API 返回的 `certificateContent` 是 **DER**，
    不是 PEM —— 直接存成 .pem 的话 openssl 会报
    「Expecting: TRUSTED CERTIFICATE / no start line」，而且报错完全
    指向不到「格式不对」这件事上。DER 的第一个字节固定是 0x30（SEQUENCE），
    据此判断即可。
    """
    import base64

    if der[:1] == b"-":
        return der
    if not der[:1] == b"\x30":
        raise SystemExit(f"既不是 PEM 也不是 DER（首字节 {der[:1]!r}）")
    # 注意：先去掉所有换行再切片。`encodebytes` 本身就按 76 字符折行，
    # 若直接对它切片，切的是「含换行的字符串」—— 结果是每行长短不一
    # （实测 64/12/51/25/38…），openssl 报 "bad end line"，而报错完全
    # 指不到「你切片切错了」这件事上。
    body = base64.b64encode(der).decode().strip()
    lines = "\n".join(body[i : i + 64] for i in range(0, len(body), 64))
    return f"-----BEGIN CERTIFICATE-----\n{lines}\n-----END CERTIFICATE-----\n".encode()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--show", action="store_true", help="只看现状，不签发")
    args = parser.parse_args()
    env = mas.load_env()
    if args.show:
        show(env)
    else:
        ensure(env)


if __name__ == "__main__":
    main()