# 内置地址集合

本目录保存 `chnroute` 自行维护的地址分类知识。它只描述不依赖地域和网络运营商数据源
的稳定集合；中国大陆及各运营商的地址仍以 `fetch` 获取的上游数据为准。

这些文件是生成策略的组成部分，不是生成结果。程序将通过 `include_str!` 把内容编译
进可执行文件，因此执行 `generate` 时不需要在运行目录中查找它们。

## 文件

| 文件 | 地址族 | 集合 |
| --- | --- | --- |
| [`private.txt`](private.txt) | IPv4 | 私有网络 |
| [`private6.txt`](private6.txt) | IPv6 | 私有网络 |
| [`special.txt`](special.txt) | IPv4 | 其他非公网地址 |
| [`special6.txt`](special6.txt) | IPv6 | 其他非公网地址 |

`private` 和 `special` 是本项目的机器可读名称。中文文档将 `special` 称为“其他非公网
地址”。

## 集合关系

本项目将每个地址族的完整地址空间分为以下几类：

```text
private = private-use address space
special = other addresses that are not ordinary globally reachable destinations
public  = universe - private - special
```

因此：

```text
private intersect special = empty
```

`special` 并不包含 `private`。采用这个名称，是为了避免 `nonpublic` 被误解为已经包含
私有地址，也避免 `non-routable` 对 CGNAT、组播等可在限定范围内转发的地址作出错误
断言。

将来生成“中国大陆以外”集合时使用：

```text
nonchina = universe - private - special - china
```

其中，`china` 完全采信上游数据，不由本目录重新判断。

## 规则语法

规则严格按照文件中的先后顺序执行：

- 空行被忽略；
- `#` 开头的行为注释；
- 普通 CIDR 将对应地址加入当前集合；
- `!CIDR` 将对应地址从当前集合中排除。

例如：

```text
192.0.0.0/24
!192.0.0.9/32
!192.0.0.10/32
```

这组规则先加入整个 `192.0.0.0/24`，再排除两个被 IANA 标记为全局可达的任播地址。
排除规则让策略文件能够直接保留注册表中的父子关系，不必把一个大网段展开成大量
CIDR。

所有 CIDR 必须满足以下要求：

- 使用规范的网络地址，主机位必须为零；
- 地址族必须与文件一致；
- 不允许在一行中放置多个 CIDR；
- `!` 必须紧邻 CIDR，不能插入空格。

## 私有网络

### IPv4

[`private.txt`](private.txt) 采用 [RFC 1918](https://www.rfc-editor.org/rfc/rfc1918)
定义的三个 IPv4 私有地址块：

```text
10.0.0.0/8
172.16.0.0/12
192.168.0.0/16
```

`100.64.0.0/10` 是 [RFC 6598](https://www.rfc-editor.org/rfc/rfc6598) 定义的共享地址
空间，主要供运营商级 NAT（CGNAT）使用，并非 RFC 1918 私有地址，因此归入
`special`。

### IPv6

[`private6.txt`](private6.txt) 采用
[RFC 4193](https://www.rfc-editor.org/rfc/rfc4193) 定义的 IPv6 唯一本地地址（ULA）：

```text
fc00::/7
```

链路本地地址、环回地址和未指定地址不属于 ULA，因此归入 `special`。

## 其他非公网地址

`special` 表示不能作为普通公网目的地址使用的地址，并明确排除 `private`。该集合不与
IANA 的 Special-Purpose Address Registry 完全等同，因为它还覆盖组播、保留空间以及
IPv6 潜在全局单播空间之外的地址。

### IPv4

[`special.txt`](special.txt) 包含以下类别：

| 地址块 | 用途 |
| --- | --- |
| `0.0.0.0/8` | 本网络及未指定用途 |
| `100.64.0.0/10` | 共享地址空间（CGNAT） |
| `127.0.0.0/8` | 环回地址 |
| `169.254.0.0/16` | 链路本地地址 |
| `192.0.0.0/24` | IETF 协议分配，但排除两个全局可达的任播地址 |
| `192.0.2.0/24` | 文档地址（TEST-NET-1） |
| `198.51.100.0/24` | 文档地址（TEST-NET-2） |
| `203.0.113.0/24` | 文档地址（TEST-NET-3） |
| `192.88.99.0/24` | 已弃用的 6to4 中继任播地址 |
| `198.18.0.0/15` | 基准测试地址 |
| `224.0.0.0/4` | 组播地址 |
| `240.0.0.0/4` | 保留地址及受限广播地址 |

`192.0.0.9/32` 和 `192.0.0.10/32` 在 IANA 注册表中的“Globally Reachable”字段为
`True`，因此通过排除规则留在 `public`。

### IPv6

[`special6.txt`](special6.txt) 以完整 IPv6 地址空间为起点，按以下步骤构造集合：

1. 加入 `::/0`。
2. 排除作为潜在全局单播空间的 `2000::/3`。
3. 排除已经归入 `private` 的 `fc00::/7`。
4. 排除 IANA 明确标记为全局可达的 IPv4—IPv6 转换前缀 `64:ff9b::/96`。
5. 加入 `2001::/23`，再排除其中被 IANA 明确标记为全局可达的更具体地址块。
6. 加入文档地址和全局可达性不明确的 6to4 地址。

IANA 将 `2000::/3` 定义为可分配的 IPv6 全局单播空间。本项目将整个地址块视为潜在
公网空间，而不根据当前 RIR 分配状态进一步拆分，避免新增分配在内置规则更新前被误判
为 `special`。

`2001::/23` 是 IETF 协议分配空间。其父级注册项并非全局可达，但下列更具体的注册项
被标记为全局可达，因此通过 `!` 规则排除：

```text
2001:1::1/128
2001:1::2/128
2001:1::3/128
2001:3::/32
2001:4:112::/48
2001:20::/28
2001:30::/28
```

以下地址块被显式加入：

- `2001:db8::/32`：文档地址；
- `3fff::/20`：文档地址；
- `2002::/16`：6to4，IANA 的全局可达性为 `N/A`。

对于“Globally Reachable”为空、`N/A` 或已经弃用的特殊用途注册项，本项目采用保守
策略，将其归入 `special`。

## 更新原则

内置知识不会在运行时联网更新。修改这些文件时应当：

1. 检查 IANA 注册表及相关 RFC 是否发生变化；
2. 明确新增或删除地址块对 `private`、`special` 和 `public` 的影响；
3. 更新文件中的注册表日期和本说明；
4. 验证 `private` 与 `special` 不相交；
5. 为新增父子覆盖关系补充边界测试；
6. 确认相同规则仍能生成确定且最小的 CIDR 集。

当前规则依据的 IANA 特殊用途地址注册表更新时间为 2025-10-09：

- [IPv4 Special-Purpose Address Space](https://www.iana.org/assignments/iana-ipv4-special-registry/iana-ipv4-special-registry.xhtml)
- [IPv6 Special-Purpose Address Space](https://www.iana.org/assignments/iana-ipv6-special-registry/iana-ipv6-special-registry.xhtml)

IPv6 全局单播空间的范围依据：

- [IPv6 Global Unicast Address Space](https://www.iana.org/assignments/ipv6-unicast-address-assignments/ipv6-unicast-address-assignments.xhtml)
