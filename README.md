# Study Proxy - HTTPS抓包工具

一个使用Rust实现的简单HTTPS抓包代理工具，类似Charles但无UI界面。

## 功能特点

- 支持HTTP/HTTPS流量拦截和分析
- JSON配置文件管理
- 自动生成和管理TLS证书
- 基于域名的过滤规则
- 详细的请求/响应日志

## 快速开始

### 1. 安装依赖

```bash
cargo build --release
```

### 配置

编辑 `config.json` 文件：

```json
{
  "proxy": {
    "host": "127.0.0.1",
    "port": 8888
  },
  "target": {
    "domains": [
      "api.github.com",
      "httpbin.org"
    ],
    "ports": [443, 8443]
  },
  "certificates": {
    "ca_cert": "certs/ca.crt",
    "ca_key": "certs/ca.key"
  },
  "logging": {
    "level": "info",
    "output": "file",
    "log_dir": "logs",
    "program_log": "proxy.log",
    "domain_logs": {
      "enabled": true,
      "format": "{date}_{domain}.log",
      "request_body_limit": 1024,
      "response_body_limit": 1024
    }
  }
}
```

### 3. 运行

```bash
# 使用默认配置文件
cargo run

# 使用自定义配置文件
cargo run -- --config my-config.json
```

### 4. 设置系统代理

将系统代理设置为 `127.0.0.1:8888`（或你在配置文件中设置的地址）。

### 5. 安装CA证书

首次运行时会自动生成CA证书，位于 `certs/ca.crt`。需要将此证书安装到系统信任存储中：

- **macOS**: 双击证书文件，添加到钥匙串，设置为始终信任
- **Windows**: 双击证书文件，安装到"受信任的根证书颁发机构"
- **Linux**: 根据发行版不同，将证书复制到相应目录

## 配置说明

### 代理设置
- `host`: 代理服务器监听地址
- `port`: 代理服务器端口

### 目标过滤
- `domains`: 要拦截的域名列表（支持子字符串匹配）
- `ports`: 要拦截的端口列表

### 证书配置
- `ca_cert`: CA证书文件路径
- `ca_key`: CA私钥文件路径

### 日志配置
- `level`: 日志级别 (error, warn, info, debug, trace)
- `output`: 日志输出位置 (stdout, file)
- `log_dir`: 日志文件目录
- `program_log`: 程序主日志文件名
- `domain_logs.enabled`: 是否启用域名日志
- `domain_logs.format`: 域名日志文件名格式
- `domain_logs.request_body_limit`: 请求体长度限制 (-1=完整记录, 0=不记录, >0=截断到指定长度)
- `domain_logs.response_body_limit`: 响应体长度限制 (-1=完整记录, 0=不记录, >0=截断到指定长度)

## 使用示例

### 拦截GitHub API请求

1. 在配置文件中添加 `api.github.com` 到 domains
2. 运行代理服务器
3. 设置系统代理为 `127.0.0.1:8888`
4. 访问GitHub，所有API请求将被记录

### 查看日志

运行时会显示类似以下日志：

```
[INFO  study_proxy] Loading configuration from: config.json
[INFO  study_proxy] Starting proxy server...
[INFO  study_proxy::proxy] Proxy server listening on 127.0.0.1:8888
[INFO  study_proxy::proxy] HTTPS CONNECT api.github.com:443
[INFO  study_proxy::proxy] GET https://api.github.com/user
[INFO  study_proxy::proxy] Response: 200 OK
```

## 注意事项

1. 确保CA证书已正确安装并信任
2. 某些应用可能需要单独配置代理设置
3. 拦截HTTPS流量需要系统级别的证书信任
4. 生产环境使用时请注意隐私和安全问题

## 开发

```bash
# 运行测试
cargo test

# 调试运行
cargo run -- --config debug-config.json

# 构建发布版本
cargo build --release
```