#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""GetCat 本地测试 server。

提供一组「难伺候」的 HTTP 接口，用来手工验证 GetCat 的慢响应处理、
大响应体渲染、流式进度与取消，以及各类异常路径。

    python3 mydata/testserver/server.py
    python3 mydata/testserver/server.py --port 9000 --host 0.0.0.0

跑起来后打开 http://127.0.0.1:8765/ 就是接口清单页。
只用标准库，无第三方依赖。所有路由对所有 HTTP method 都生效。
"""

from __future__ import annotations

import argparse
import html
import json
import re
import socket
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

# 一次写给 socket 的最大块，也是流式生成响应体的粒度。
# 取 64 KB：足够摊薄 syscall 开销，又不至于让「取消」要等很久才生效。
WRITE_CHUNK = 64 * 1024

# 单个响应体的上限，纯粹是防手滑（比如把 50mb 敲成 50gb）。
MAX_BODY = 512 * 1024 * 1024

# /slow 的上限，防止把 server 的线程永久占死。
MAX_DELAY = 300.0


class BadParam(Exception):
    """参数不合法，直接回 400。"""


# ── 参数解析 ────────────────────────────────────────────────────────

_SIZE_RE = re.compile(r"^\s*(\d+(?:\.\d+)?)\s*(b|kb?|mb?|gb?)?\s*$", re.IGNORECASE)
_UNIT = {
    "": 1, "b": 1,
    "k": 1024, "kb": 1024,
    "m": 1024 ** 2, "mb": 1024 ** 2,
    "g": 1024 ** 3, "gb": 1024 ** 3,
}


def parse_size(raw, default):
    """把 '5mb' / '512kb' / '1048576' 解析成字节数。"""
    if raw is None or raw == "":
        return default
    m = _SIZE_RE.match(raw)
    if not m:
        raise BadParam("size 无法识别：%r（试试 1mb / 512kb / 1048576）" % raw)
    total = int(float(m.group(1)) * _UNIT[(m.group(2) or "").lower()])
    if not 0 <= total <= MAX_BODY:
        raise BadParam("size 必须在 0 到 %d 字节（512mb）之间，收到 %d" % (MAX_BODY, total))
    return total


def parse_float(raw, default, lo, hi, name):
    if raw is None or raw == "":
        return default
    try:
        value = float(raw)
    except ValueError:
        raise BadParam("%s 必须是数字，收到 %r" % (name, raw)) from None
    if not lo <= value <= hi:
        raise BadParam("%s 必须在 %s 到 %s 之间，收到 %s" % (name, lo, hi, value))
    return value


def parse_int(raw, default, lo, hi, name):
    if raw is None or raw == "":
        return default
    try:
        value = int(raw, 10)
    except ValueError:
        raise BadParam("%s 必须是整数，收到 %r" % (name, raw)) from None
    if not lo <= value <= hi:
        raise BadParam("%s 必须在 %d 到 %d 之间，收到 %d" % (name, lo, hi, value))
    return value


def parse_bool(raw):
    return (raw or "").lower() in ("1", "true", "yes", "on")


# ── 响应体的内容形态 ────────────────────────────────────────────────
#
# 「大响应体」里到底装什么，直接决定你测出来的是什么。三种取舍：
#
#   · 重复同一行     —— 生成最快，但压缩率虚高，也不像真实数据
#   · 随机字节       —— 最真实，但生成 50 MB 会明显拖慢 server 自己
#   · 递增 id 的记录 —— 当前实现。顺带能测 JSON 高亮与折叠，代价是
#                       客户端的 JSON 解析开销会混进耗时里
#
# 想换形态就改这一节。下面的布局算法只依赖两个约定：
#   1. 定长记录的字节数（JSON_RECORD_LEN / TEXT_LINE_LEN）必须准确；
#   2. 尾部能用一条任意长度的填充记录补齐，好让总字节数正好等于请求值。
# 只要守住这两条，Content-Length 就不会和实际写出的字节对不上。

FILLER = "GetCat"


def _filler(n):
    """长度恰好为 n 的填充串。用可见字符，方便肉眼确认没被截断。"""
    if n <= 0:
        return ""
    return (FILLER * (n // len(FILLER) + 1))[:n]


JSON_PAYLOAD_LEN = 96
JSON_RECORD_LEN = 32 + JSON_PAYLOAD_LEN          # '  {"id":"%08d","payload":"…"}' = 128
JSON_TAIL_MIN = len('  {"pad":""}')              # 收尾记录的最小长度 = 12
_JSON_PAYLOAD = _filler(JSON_PAYLOAD_LEN)        # 预先算好，50 MB 要用它四十万次

TEXT_LINE_LEN = 128                              # 含行尾换行
_TEXT_PAYLOAD = _filler(TEXT_LINE_LEN - 15)      # 'line ' + 8 位 id + ' ' + payload + '\n'


def _json_record(index):
    # id 用字符串而不是数字：定长要靠 %08d 补零，但 JSON 不允许 00000001
    # 这种前导零的数字字面量，直接写成数字会生成解析不了的「假 JSON」。
    return '  {"id":"%08d","payload":"%s"},\n' % (index, _JSON_PAYLOAD)


def _json_tail(length):
    """长度恰好为 length 的收尾记录，length 必须 >= JSON_TAIL_MIN。"""
    return '  {"pad":"%s"}' % _filler(length - JSON_TAIL_MIN)


def _text_line(index):
    return "line %08d %s\n" % (index, _TEXT_PAYLOAD)


def _gen_json(total):
    """产出总长恰好 total 字节的 JSON 数组。"""
    prefix, suffix = b"[\n", b"\n]\n"
    budget = total - len(prefix) - len(suffix)
    step = JSON_RECORD_LEN + 2                   # 定长记录 + ",\n"
    count = max(0, (budget - JSON_TAIL_MIN) // step)
    tail = budget - count * step                 # 恒 >= JSON_TAIL_MIN

    buf = bytearray(prefix)
    for i in range(1, count + 1):
        buf += _json_record(i).encode("ascii")
        if len(buf) >= WRITE_CHUNK:
            yield bytes(buf)
            buf.clear()
    buf += _json_tail(tail).encode("ascii")
    buf += suffix
    yield bytes(buf)


def _gen_text(total):
    """产出总长恰好 total 字节的纯文本。"""
    count, rem = divmod(total, TEXT_LINE_LEN)
    buf = bytearray()
    for i in range(1, count + 1):
        buf += _text_line(i).encode("ascii")
        if len(buf) >= WRITE_CHUNK:
            yield bytes(buf)
            buf.clear()
    if rem:
        buf += (_filler(rem - 1) + "\n").encode("ascii")
    if buf:
        yield bytes(buf)


def iter_body(total, kind):
    """按 kind 返回一个产出 total 字节的生成器。参数错误立刻抛，不拖到发完 header。"""
    if kind == "json":
        floor = len(b"[\n") + len(b"\n]\n") + JSON_TAIL_MIN
        if total < floor:
            raise BadParam("type=json 至少需要 %d 字节，收到 %d（小响应体请用 type=text）" % (floor, total))
        return _gen_json(total)
    if kind == "text":
        return _gen_text(total)
    raise BadParam("type 只支持 json / text，收到 %r" % kind)


def content_type_for(kind):
    return "application/json; charset=utf-8" if kind == "json" else "text/plain; charset=utf-8"


# ── 接口清单（索引页与终端提示都由它生成，加接口只改这一处）──────────

ROUTES_DOC = [
    {
        "path": "/slow",
        "name": "慢响应",
        "desc": "先挂起 delay 秒再返回。用来看等待态、计时器和「取消」按钮是否好使。"
                "带上 size 就变成「又慢又大」。",
        "params": [
            ("delay", "3", "延迟秒数，0 – 300"),
            ("size", "0", "可选：延迟后返回多大的体，如 5mb"),
            ("type", "json", "size > 0 时的内容形态：json / text"),
        ],
        "examples": ["/slow?delay=3", "/slow?delay=30", "/slow?delay=5&size=10mb"],
    },
    {
        "path": "/big",
        "name": "超大响应体",
        "desc": "一次性返回指定大小，带准确的 Content-Length，进度条能算百分比。"
                "server 端按 64 KB 分块写出，自己不占内存。",
        "params": [
            ("size", "5mb", "1mb / 5mb / 10mb / 20mb / 50mb，或任意字节数，上限 512mb"),
            ("type", "json", "json＝合法 JSON 数组（能测高亮与折叠）；text＝纯文本行"),
            ("chunked", "0", "填 1 则改用 chunked 传输、不给 Content-Length，测「总量未知」时的进度显示"),
        ],
        "examples": [
            "/big?size=1mb", "/big?size=5mb", "/big?size=10mb",
            "/big?size=20mb", "/big?size=50mb",
            "/big?size=5mb&type=text", "/big?size=10mb&chunked=1",
        ],
    },
    {
        "path": "/stream",
        "name": "流式滴流",
        "desc": "chunked 传输，每隔 interval 秒吐一块，共 chunks 块。"
                "专门测实时进度和中途取消——一次性的大响应体是测不出进度条的。",
        "params": [
            ("chunks", "20", "块数，1 – 10000"),
            ("interval", "0.2", "每块之间的间隔秒数，0 – 60"),
            ("chunk_size", "64kb", "每块大小"),
        ],
        "examples": [
            "/stream?chunks=20&interval=0.2",
            "/stream?chunks=100&interval=0.5&chunk_size=256kb",
            "/stream?chunks=10&interval=3",
        ],
    },
    {
        "path": "/status/<code>",
        "name": "任意状态码",
        "desc": "返回指定的状态码和一段 JSON 错误体。204 与 304 按规范不带 body。",
        "params": [
            ("<code>", "—", "路径参数，200 – 599"),
        ],
        "examples": ["/status/201", "/status/204", "/status/400", "/status/404", "/status/500", "/status/503"],
    },
    {
        "path": "/abort",
        "name": "中途断连",
        "desc": "声明一个很大的 Content-Length，只写出一部分就直接关掉 socket。"
                "客户端会在读取途中撞上 EOF——测半截响应的处理，不是测一个正常结束的短响应。",
        "params": [
            ("declare", "10mb", "响应头里声明的 Content-Length"),
            ("after", "1mb", "实际写出多少字节后断开，需 <= declare"),
        ],
        "examples": ["/abort", "/abort?declare=50mb&after=5mb", "/abort?declare=1mb&after=0"],
    },
    {
        "path": "/headers",
        "name": "超多超长响应头",
        "desc": "返回一堆又多又长的自定义响应头，测 header 列表的渲染与折行。"
                "注意多数客户端对响应头总量有上限（curl 是 300 KB，超了直接断开），"
                "最后一个示例就是故意越过这条线的。",
        "params": [
            ("count", "50", "响应头数量，1 – 500"),
            ("size", "1kb", "每个头的值有多长，上限 64kb"),
        ],
        "examples": ["/headers", "/headers?count=200&size=1kb", "/headers?count=5&size=32kb",
                     "/headers?count=200&size=2kb"],
    },
    {
        "path": "/empty",
        "name": "空响应体",
        "desc": "200 + Content-Length: 0。测空 body 时界面别显示成「加载中」或报错。",
        "params": [],
        "examples": ["/empty"],
    },
]


# ── 索引页 ──────────────────────────────────────────────────────────

_INDEX_HEAD = """<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>GetCat 测试接口</title>
<style>
:root {
  color-scheme: light dark;
  --bg: #fbfbfd; --fg: #1d1d1f; --muted: #6e6e73;
  --card: #ffffff; --line: #e5e5ea; --accent: #0071e3; --code-bg: #f2f2f7;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #161618; --fg: #f5f5f7; --muted: #98989d;
    --card: #1f1f22; --line: #333338; --accent: #4a9eff; --code-bg: #2a2a2e;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; padding: 48px 24px 80px; background: var(--bg); color: var(--fg);
  font: 15px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC",
        "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
}
.wrap { max-width: 940px; margin: 0 auto; }
h1 { font-size: 30px; letter-spacing: -.02em; margin: 0 0 8px; }
.lede { color: var(--muted); margin: 0 0 8px; }
.base { color: var(--muted); margin: 0 0 40px; font-size: 14px; }
.card {
  background: var(--card); border: 1px solid var(--line); border-radius: 14px;
  padding: 22px 24px; margin-bottom: 18px;
}
.card h2 { font-size: 18px; margin: 0 0 2px; letter-spacing: -.01em; }
.card h2 .route {
  font: 500 14px/1 ui-monospace, SFMono-Regular, Menlo, monospace;
  color: var(--accent); margin-left: 10px;
}
.card p { color: var(--muted); margin: 6px 0 16px; }
table { width: 100%; border-collapse: collapse; margin-bottom: 16px; }
th, td { text-align: left; padding: 7px 10px 7px 0; border-bottom: 1px solid var(--line); vertical-align: top; }
th { font-size: 12px; font-weight: 600; color: var(--muted); text-transform: uppercase; letter-spacing: .04em; }
td:first-child, th:first-child { width: 120px; }
td:nth-child(2), th:nth-child(2) { width: 90px; }
code {
  font: 13px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
  background: var(--code-bg); padding: 1px 6px; border-radius: 5px;
}
.ex { display: flex; flex-wrap: wrap; gap: 8px; }
.ex .item { display: flex; align-items: stretch; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
.ex a {
  font: 13px/1 ui-monospace, SFMono-Regular, Menlo, monospace;
  color: var(--fg); text-decoration: none; padding: 8px 10px; background: var(--code-bg);
}
.ex a:hover { color: var(--accent); }
.ex button {
  border: 0; border-left: 1px solid var(--line); background: transparent; cursor: pointer;
  color: var(--muted); font-size: 12px; padding: 0 10px; font-family: inherit; white-space: nowrap;
}
.ex button:hover { color: var(--accent); }
.note { color: var(--muted); font-size: 13px; margin-top: 40px; }
.note code { font-size: 12px; }
</style>
</head>
<body>
<div class="wrap">
<h1>GetCat 测试接口</h1>
<p class="lede">一组专门用来为难客户端的接口：慢响应、超大响应体、流式滴流、错误码与异常。</p>
<p class="base">当前地址 <code id="base">—</code> ·
点接口名直接在浏览器里打开，点「复制」把完整 URL 拿去粘到 GetCat。</p>
"""

_INDEX_TAIL = """<p class="note">
提示：<code>/big?size=50mb</code> 这类链接在浏览器里点开会真的下载 50 MB，拿去 GetCat 里测更合适。<br>
所有接口对 GET / POST / PUT / PATCH / DELETE / HEAD / OPTIONS 都生效，换 method 不用换 URL；请求体会被读完并丢弃。<br>
命令行核对用 <code>curl -s -o /dev/null -w '%{size_download} bytes in %{time_total}s\\n' "URL"</code>。
</p>
</div>
<script>
document.getElementById('base').textContent = location.origin;
document.addEventListener('click', function (e) {
  const btn = e.target.closest('button[data-copy]');
  if (!btn) return;
  navigator.clipboard.writeText(location.origin + btn.dataset.copy).then(function () {
    const old = btn.textContent;
    btn.textContent = '已复制';
    setTimeout(function () { btn.textContent = old; }, 1200);
  });
});
</script>
</body>
</html>
"""


def render_index():
    out = [_INDEX_HEAD]
    for route in ROUTES_DOC:
        out.append('<div class="card">\n')
        out.append('<h2>%s<span class="route">%s</span></h2>\n'
                   % (html.escape(route["name"]), html.escape(route["path"])))
        out.append("<p>%s</p>\n" % html.escape(route["desc"]))
        if route["params"]:
            out.append("<table><tr><th>参数</th><th>默认</th><th>说明</th></tr>\n")
            for name, default, desc in route["params"]:
                out.append("<tr><td><code>%s</code></td><td><code>%s</code></td><td>%s</td></tr>\n"
                           % (html.escape(name), html.escape(default), html.escape(desc)))
            out.append("</table>\n")
        out.append('<div class="ex">\n')
        for example in route["examples"]:
            safe = html.escape(example, quote=True)
            out.append('<span class="item"><a href="%s">%s</a>'
                       '<button data-copy="%s">复制</button></span>\n' % (safe, safe, safe))
        out.append("</div>\n</div>\n")
    out.append(_INDEX_TAIL)
    return "".join(out)


# ── 请求处理 ────────────────────────────────────────────────────────

class TestHandler(BaseHTTPRequestHandler):
    # 必须是 HTTP/1.1，否则 chunked 传输和 keep-alive 都不生效。
    # 代价是每个响应都得给出准确的 Content-Length 或走 chunked。
    protocol_version = "HTTP/1.1"
    server_version = "GetCatTestServer/1.0"
    sys_version = ""

    # ---- 框架层容错 ----

    def handle_one_request(self):
        # 客户端点了「取消」、或 /abort 主动关掉了 socket 之后，
        # 标准库还会去 flush 一次 wfile。这里兜住，别刷一屏 traceback。
        try:
            super().handle_one_request()
        except (BrokenPipeError, ConnectionResetError, OSError, ValueError):
            self.close_connection = True

    def finish(self):
        try:
            super().finish()
        except (OSError, ValueError):
            pass

    # ---- 路由 ----

    def _dispatch(self):
        parsed = urlparse(self.path)
        path = parsed.path.rstrip("/") or "/"
        self.query = {k: v[0] for k, v in parse_qs(parsed.query, keep_blank_values=True).items()}
        self._drain_request_body()
        try:
            endpoint = self._route(path)
            if endpoint is None:
                self.send_json(404, {
                    "error": "no such endpoint",
                    "path": path,
                    "hint": "打开 / 看接口清单",
                })
                return
            endpoint()
        except BadParam as exc:
            self.send_json(400, {"error": "bad parameter", "detail": str(exc)})
        except (BrokenPipeError, ConnectionResetError):
            self.log_message("客户端提前断开：%s", self.path)
            self.close_connection = True

    # 所有 method 走同一套处理，这样在 GetCat 里换 method 手测时不用换 URL。
    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_HEAD = do_OPTIONS = _dispatch

    def _route(self, path):
        if path == "/":
            return self.ep_index
        if path.startswith("/status/"):
            return self.ep_status
        return {
            "/slow": self.ep_slow,
            "/big": self.ep_big,
            "/stream": self.ep_stream,
            "/abort": self.ep_abort,
            "/headers": self.ep_headers,
            "/empty": self.ep_empty,
        }.get(path)

    def _drain_request_body(self):
        """请求体读完丢弃。keep-alive 下不读干净，下一个请求就会解析到残留字节。"""
        try:
            length = self.headers.get("Content-Length")
            if length:
                remaining = int(length)
                while remaining > 0:
                    data = self.rfile.read(min(remaining, WRITE_CHUNK))
                    if not data:
                        break
                    remaining -= len(data)
            elif "chunked" in (self.headers.get("Transfer-Encoding") or "").lower():
                while True:
                    line = self.rfile.readline()
                    if not line:
                        break
                    size = int(line.strip().split(b";")[0] or b"0", 16)
                    if size == 0:
                        self.rfile.readline()
                        break
                    self.rfile.read(size)
                    self.rfile.readline()
        except (ValueError, OSError):
            self.close_connection = True

    # ---- 响应工具 ----

    def _is_head(self):
        return self.command == "HEAD"

    def start(self, code, content_type, length=None, extra=None, chunked=False):
        if self._is_head():
            # HEAD 不该有 body；chunked 接口没法预先算出长度，就报 0。
            chunked = False
            length = 0 if length is None else length
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        if chunked:
            self.send_header("Transfer-Encoding", "chunked")
        elif length is not None:
            self.send_header("Content-Length", str(length))
        for key, value in extra or ():
            self.send_header(key, value)
        self.end_headers()

    def send_json(self, code, payload, extra=None):
        body = json.dumps(payload, ensure_ascii=False, indent=2).encode("utf-8")
        self.start(code, "application/json; charset=utf-8", len(body), extra)
        if not self._is_head():
            self.wfile.write(body)

    def write_plain(self, chunks):
        """按 Content-Length 模式写出，chunks 的总长必须和已声明的一致。"""
        if self._is_head():
            return 0
        written = 0
        for chunk in chunks:
            self.wfile.write(chunk)
            written += len(chunk)
        return written

    def write_chunked(self, chunks):
        """自己编 chunked 帧：十六进制长度 + CRLF + 数据 + CRLF，末尾一个 0 帧。"""
        if self._is_head():
            return 0
        written = 0
        for chunk in chunks:
            self.wfile.write(b"%X\r\n" % len(chunk))
            self.wfile.write(chunk)
            self.wfile.write(b"\r\n")
            self.wfile.flush()
            written += len(chunk)
        self.wfile.write(b"0\r\n\r\n")
        return written

    # ---- 各接口 ----

    def ep_index(self):
        body = render_index().encode("utf-8")
        self.start(200, "text/html; charset=utf-8", len(body))
        if not self._is_head():
            self.wfile.write(body)

    def ep_slow(self):
        delay = parse_float(self.query.get("delay"), 3.0, 0.0, MAX_DELAY, "delay")
        size = parse_size(self.query.get("size"), 0)
        kind = (self.query.get("type") or "json").lower()
        chunks = iter_body(size, kind) if size else None   # 先校验，再睡

        began = time.monotonic()
        time.sleep(delay)
        elapsed = round(time.monotonic() - began, 3)

        if chunks is not None:
            self.start(200, content_type_for(kind), size,
                       extra=[("X-GetCat-Delay-Seconds", str(delay))])
            self.write_plain(chunks)
        else:
            self.send_json(200, {
                "endpoint": "/slow",
                "delay_seconds": delay,
                "slept_seconds": elapsed,
                "method": self.command,
            })

    def ep_big(self):
        size = parse_size(self.query.get("size"), 5 * 1024 * 1024)
        kind = (self.query.get("type") or "json").lower()
        chunked = parse_bool(self.query.get("chunked"))
        chunks = iter_body(size, kind)
        extra = [("X-GetCat-Body-Bytes", str(size))]
        if chunked:
            self.start(200, content_type_for(kind), extra=extra, chunked=True)
            self.write_chunked(chunks)
        else:
            self.start(200, content_type_for(kind), size, extra=extra)
            self.write_plain(chunks)

    def ep_stream(self):
        count = parse_int(self.query.get("chunks"), 20, 1, 10000, "chunks")
        interval = parse_float(self.query.get("interval"), 0.2, 0.0, 60.0, "interval")
        chunk_size = parse_size(self.query.get("chunk_size"), 64 * 1024)
        if chunk_size < 1:
            raise BadParam("chunk_size 至少 1 字节")
        if count * chunk_size > MAX_BODY:
            raise BadParam("chunks × chunk_size = %d 字节，超过上限 %d" % (count * chunk_size, MAX_BODY))
        self.start(200, "text/plain; charset=utf-8", chunked=True, extra=[
            ("X-GetCat-Total-Chunks", str(count)),
            ("X-GetCat-Chunk-Size", str(chunk_size)),
            ("X-GetCat-Total-Bytes", str(count * chunk_size)),
        ])
        self.write_chunked(self._drip(count, interval, chunk_size))

    def _drip(self, count, interval, chunk_size):
        for i in range(1, count + 1):
            if i > 1 and interval:
                time.sleep(interval)
            label = "chunk %04d/%04d " % (i, count)
            if chunk_size > len(label):
                label += _filler(chunk_size - len(label) - 1) + "\n"
            yield label.encode("ascii")[:chunk_size]

    def ep_status(self):
        raw = urlparse(self.path).path.rstrip("/").rsplit("/", 1)[-1]
        try:
            code = int(raw, 10)
        except ValueError:
            raise BadParam("状态码必须是整数，收到 %r" % raw) from None
        if not 200 <= code <= 599:
            raise BadParam("只支持 200 – 599，收到 %d（1xx 会让客户端一直等最终响应，这里不做）" % code)
        if code in (204, 304):
            # 这两个按规范不能带 body，也不该带 Content-Length。
            self.send_response(code)
            self.end_headers()
            return
        self.send_json(code, {
            "endpoint": "/status/%d" % code,
            "status": code,
            "reason": self.responses.get(code, ("Unknown",))[0] if code in self.responses else "Unknown",
            "method": self.command,
        })

    def ep_abort(self):
        declare = parse_size(self.query.get("declare"), 10 * 1024 * 1024)
        after = parse_size(self.query.get("after"), 1 * 1024 * 1024)
        if after > declare:
            raise BadParam("after (%d) 不能大于 declare (%d)" % (after, declare))
        chunks = iter_body(declare, "json")

        self.start(200, "application/json; charset=utf-8", declare,
                   extra=[("X-GetCat-Will-Abort-After", str(after))])
        if not self._is_head():
            sent = 0
            for chunk in chunks:
                if sent + len(chunk) >= after:
                    self.wfile.write(chunk[: after - sent])
                    sent = after
                    break
                self.wfile.write(chunk)
                sent += len(chunk)
            try:
                self.wfile.flush()
            except (OSError, ValueError):
                pass
        self.log_message("声明 %d 字节，写出 %d 字节后主动断开", declare, after)
        self.close_connection = True
        try:
            self.connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.connection.close()

    def ep_headers(self):
        count = parse_int(self.query.get("count"), 50, 1, 500, "count")
        size = parse_size(self.query.get("size"), 1024)
        if not 1 <= size <= 64 * 1024:
            raise BadParam("每个响应头的值需在 1 字节到 64kb 之间，收到 %d" % size)
        extra = [("X-GetCat-Test-%03d" % i, _filler(size)) for i in range(1, count + 1)]
        self.send_json(200, {
            "endpoint": "/headers",
            "header_count": count,
            "value_bytes": size,
            "total_header_bytes": count * (size + len("X-GetCat-Test-000: \r\n")),
        }, extra=extra)

    def ep_empty(self):
        self.start(200, "application/json; charset=utf-8", 0)

    # ---- 日志 ----

    def log_message(self, fmt, *args):
        print("[%s] %s - %s" % (time.strftime("%H:%M:%S"), self.address_string(), fmt % args), flush=True)


# ── 入口 ────────────────────────────────────────────────────────────

def main(argv=None):
    parser = argparse.ArgumentParser(
        description="GetCat 本地测试 server：慢响应、超大响应体、流式滴流、错误码与异常。")
    parser.add_argument("--host", default="127.0.0.1",
                        help="监听地址，默认 127.0.0.1；想让同网段的手机/别的机器访问就用 0.0.0.0")
    parser.add_argument("--port", type=int, default=8765, help="监听端口，默认 8765")
    args = parser.parse_args(argv)

    server = ThreadingHTTPServer((args.host, args.port), TestHandler)
    server.daemon_threads = True   # Ctrl-C 时不等慢请求做完

    shown = "127.0.0.1" if args.host in ("", "0.0.0.0", "::") else args.host
    base = "http://%s:%d" % (shown, server.server_address[1])
    print("GetCat 测试 server 已启动")
    print("  接口清单页  %s/" % base)
    for route in ROUTES_DOC:
        print("  %-14s %s" % (route["path"], route["name"]))
    print("Ctrl-C 退出。\n")

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n已停止。")
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
