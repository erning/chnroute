# chnroute

`chnroute` 用于获取并生成按区域和网络运营商划分的 IPv4、IPv6 路由表。

`fetch` 子命令从
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

## 生成路由表

获取原始数据后，运行：

```console
cargo run -- generate
```

`generate` 默认读取 `data/raw`，并将结果写入 `dist`。也可以分别指定输入和输出目录：

```console
cargo run -- generate --input data/raw --output dist
```

生成过程不会访问网络，也不会修改原始快照。程序首先根据 `manifest.json` 验证全部
16 个原始文件的大小和 SHA-256，然后执行集合运算。输入不完整或内容与清单不符时，
命令会停止，不会发布部分结果。

### 输出文件

每个集合分别生成 IPv4 和 IPv6 文件。IPv4 使用基本文件名，IPv6 在扩展名前增加
`6`：

| 集合 | IPv4 | IPv6 |
| --- | --- | --- |
| 中国大陆 | `chnroute.txt` | `chnroute6.txt` |
| 中国大陆以外的公网地址 | `non-chnroute.txt` | `non-chnroute6.txt` |
| 中国电信 | `china-telecom.txt` | `china-telecom6.txt` |
| 中国移动 | `china-mobile.txt` | `china-mobile6.txt` |
| 中国联通 | `china-unicom.txt` | `china-unicom6.txt` |
| 中国其他运营商 | `china-other.txt` | `china-other6.txt` |
| 私有网络 | `private.txt` | `private6.txt` |
| 其他非公网地址 | `special.txt` | `special6.txt` |

三个组合集合的定义如下：

```ini
chnroute     = public addresses in mainland China
non-chnroute = public addresses outside chnroute
china-other  = other mainland China operators
```

`china-other` 是教育网、科技网、鹏博士和谷歌中国四个上游集合的并集。运营商集合可能
相互重叠，生成过程不会自行规定运营商优先级，也不会从 `china-other` 中扣除三大运营商
的地址。

`non-chnroute` 按以下关系计算：

```text
non-chnroute = complete address space - private - special - chnroute
```

其中，`private` 和 `special` 来自编译进程序的
[`data/builtin`](data/builtin/README.md) 规则，二者互不相交。`chnroute` 直接采用上游
`china` 集合，不使用本地归属判断。

所有输出都会按照地址顺序排列，并合并为确定且最小的 CIDR 集。`dist/manifest.json`
记录输入仓库、提交散列，以及每个输出文件的地址族、CIDR 数量、文件大小和 SHA-256。

生成结果同样通过暂存目录整体发布。程序可以安全替换自己此前生成的目录，但不会替换
符号链接、与输入目录重叠的目录，或缺少兼容 `manifest.json` 的非空目录。

## 发布到 `dist` 分支

运行以下脚本，可以重新生成路由表，并将 `dist/` 的内容发布到本地 `dist` 分支的根
目录：

```console
scripts/publish-dist.sh
```

脚本要求源码工作树没有未提交修改。发布前会离线运行测试和 `generate`，然后在由
`git worktree` 创建的临时工作树中维护发行分支。首次发布时，脚本将 `dist` 创建为
不包含源码历史的孤立分支；后续发布提交沿用该分支现有历史。脚本不会切换当前工作树的
分支，也不会删除当前工作树中的源码。

发行分支的根目录与本地 `dist/` 完全一致，只包含生成的路由表和 `manifest.json`。
如果生成内容没有变化，脚本不会创建空提交。

默认操作只创建本地提交，不访问远端。确认结果后，可以显式推送到 `origin`：

```console
scripts/publish-dist.sh --push
```

每个发行提交都会在提交信息中记录生成它的源码提交。上游数据提交、文件散列和前缀
数量继续以 `manifest.json` 为准。如果远端 `dist` 已经前进，普通推送会安全失败；脚本
不会自动拉取、合并或强制推送。

发布到 GitHub 后，可以通过固定的原始文件 URL 订阅根目录中的文件：

```text
https://raw.githubusercontent.com/<owner>/<repository>/dist/chnroute.txt
```

## 测试

```console
cargo test
```

`fetch` 测试使用内存中的模拟响应，`generate` 测试使用临时快照；测试过程不访问网络。
