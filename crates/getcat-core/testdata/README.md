# 测试用证书

`tls::inspect` 的体检结论依赖有效期，用现生成的证书会随时间漂移。这里的三张
自签名证书把 `notBefore` / `notAfter` 写死在过去或未来，结论因此永久稳定。

重新生成（OpenSSL 3.x，`-not_before` / `-not_after` 需要 3.0 以上）：

```sh
gen() {
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
    -keyout /dev/null -outform DER -out "$1" \
    -subj "/CN=$2" -addext "subjectAltName=$3" \
    -not_before "$4" -not_after "$5"
}
gen expired.der       localhost "DNS:localhost"                   20200101000000Z 20210101000000Z
gen not-yet-valid.der localhost "DNS:localhost"                   21000101000000Z 21010101000000Z
gen self-signed.der   localhost "DNS:localhost,DNS:*.example.com" 20200101000000Z 21000101000000Z
```

私钥直接丢弃（`-keyout /dev/null`）：这些证书只用来喂解析器，永远不参与握手。
