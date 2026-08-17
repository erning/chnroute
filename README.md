# chnroute

`chnroute` 用于获取并生成按区域和网络运营商划分的 IPv4、IPv6 路由表。

当前仅实现 `fetch` 子命令。该命令从
[`gaoyifan/china-operator-ip`](https://github.com/gaoyifan/china-operator-ip)
的 `ip-lists` 分支下载以下原始数据：

- 中国大陆；
- 中国电信；
- 中国移动；
- 中国联通；
- 教育网；
- 科技网；
- 鹏博士；
- 谷歌中国。

每类数据分别下载 IPv4 和 IPv6 文件，共 16 个文件。

## 构建

```console
cargo build --release
```

## 获取原始数据

```console
cargo run -- fetch
```

默认将数据写入 `data/raw`：

```console
cargo run -- fetch --output data/raw
```

可以指定上游分支、标签或提交散列：

```console
cargo run -- fetch --ref ip-lists
cargo run -- fetch --ref <commit-sha>
```

如果当前目录已经包含同一提交的完整快照，命令不会重复下载。使用 `--force`
可以强制重新下载：

```console
cargo run -- fetch --force
```

## 数据完整性

`fetch` 首先通过 Git Smart HTTP 协议将分支或标签解析为确定的 Git 提交散列，然后从
该提交下载全部文件，避免一次构建混入不同时间的上游数据。解析过程不使用 GitHub
REST API，不受未认证 API 请求额度影响。指定完整的 40 位提交散列时，会直接下载该
提交的数据。

下载完成后，程序会验证：

- 每个非空行都是规范的 CIDR；
- IPv4、IPv6 地址与文件类型一致；
- 响应大小不超过安全限制。

没有对应地址前缀时，上游文件可以为空。例如，某个试验阶段运营商可能暂时没有
IPv6 地址；此时 `manifest.json` 中的 CIDR 数量为 `0`。

原始文件的字节不会被排序、合并或改写。`manifest.json` 记录上游仓库、请求的引用、
实际提交、获取时间、地址族、CIDR 数量、文件大小和 SHA-256。

新快照会先写入暂存目录。只有全部 16 个文件成功下载并通过校验后，程序才会替换现有
快照；失败时保留原有数据。为避免误删用户文件，程序不会替换缺少有效
`manifest.json` 的非空目录，也不会替换符号链接。

## 测试

```console
cargo test
```

测试使用内存中的模拟响应，不访问网络。
