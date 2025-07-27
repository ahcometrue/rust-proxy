# HTTPS代理证书使用指南

## 自动生成的证书

运行程序后，会自动在 `certs/` 目录下生成以下文件：

### 1. CA根证书 (ca.crt)
- **文件**: `certs/ca.crt`
- **用途**: 作为根CA证书，用于签发站点证书
- **有效期**: 50年
- **算法**: ECDSA P-256

### 2. CA私钥 (ca.key)
- **文件**: `certs/ca.key`
- **用途**: CA证书的私钥，用于签发站点证书
- **重要**: 请妥善保管，不要泄露

## 自动配置功能

程序现在内置了自动配置功能，无需手动执行bash脚本：

### 1. 证书自动安装
启动程序时会自动将CA证书安装到系统信任存储：
- **macOS**: 自动添加到系统钥匙串
- **Linux**: 自动配置系统证书存储
- **Windows**: 自动添加到受信任的根证书颁发机构

### 2. curl环境自动配置
程序会自动配置curl环境，无需手动设置：
- 自动创建 `~/.curlrc` 文件，包含代理和CA证书配置
- 自动设置环境变量：`HTTP_PROXY`、`HTTPS_PROXY`、`CURL_CA_BUNDLE`
- 自动配置shell环境（.zshrc/.bashrc）

## 使用方法

### 1. 启动代理
```bash
cargo run -- --config config.json
```

### 2. 自动配置完成
启动后，程序会自动：
- 生成证书文件
- 安装证书到系统信任存储
- 配置curl环境
- 设置系统代理（如果启用）

### 3. 验证配置
测试curl命令，无需任何额外参数：
```bash
curl https://api.github.com
```

## 配置文件说明

编辑 `config.json` 来自定义行为：

```json
{
  "proxy": {
    "host": "127.0.0.1",
    "port": 8888
  },
  "target": {
    "domains": ["api.github.com", "httpbin.org"],
    "ports": [443, 8443]
  },
  "certificates": {
    "ca_cert": "certs/ca.crt",
    "ca_key": "certs/ca.key",
    "auto_install": true,
    "auto_uninstall": false,
    "name": "rustProxyCA",
    "configure_curl": true
  },
  "system_proxy": {
    "enabled": true,
    "auto_configure": true
  }
}
```

### 配置选项说明

- **certificates.auto_install**: 是否自动安装证书到系统信任存储
- **certificates.auto_uninstall**: 程序关闭时是否自动卸载证书
- **certificates.configure_curl**: 是否自动配置curl环境
- **system_proxy.enabled**: 是否启用系统代理
- **system_proxy.auto_configure**: 是否自动配置系统代理设置

## 手动验证

### 1. 检查证书信息
```bash
openssl x509 -in certs/ca.crt -text -noout
```

### 2. 测试HTTPS连接
```bash
# 无需额外参数，curl会自动使用配置
curl https://api.github.com

# 检查代理是否工作
curl -v https://httpbin.org/ip
```

### 3. 检查curl配置
```bash
# 查看curl使用的配置文件
echo $CURL_CA_BUNDLE

# 查看环境变量
echo $HTTP_PROXY
echo $HTTPS_PROXY
```

## 清理配置

程序关闭时会自动清理：
- 移除curl环境配置
- 清理shell环境变量（如果auto_uninstall为true）
- 卸载系统证书（如果auto_uninstall为true）

### 手动清理
如果需要手动清理：
```bash
# 删除curl配置文件
rm ~/.curlrc

# 从shell配置文件中移除相关配置
# 编辑 ~/.zshrc 或 ~/.bashrc，删除study-proxy相关行
```

## 常见问题

### 1. 证书无效警告
- 确保程序已正确启动并完成自动配置
- 重启浏览器或系统
- 检查证书是否过期

### 2. 连接被拒绝
- 确认代理服务器正在运行
- 检查防火墙设置
- 验证端口是否被占用

### 3. 证书生成失败
- 删除 `certs/` 目录重新生成
- 检查文件权限
- 确保磁盘空间充足

## 平台特定说明

### macOS
- 证书会自动添加到系统钥匙串
- 需要管理员权限进行系统证书安装
- 支持zsh和bash shell配置

### Linux
- 支持Ubuntu/Debian的`update-ca-certificates`
- 支持CentOS/RHEL的`update-ca-trust`
- 支持bash和zsh shell配置

### Windows
- 证书会自动添加到受信任的根证书颁发机构
- 需要管理员权限
- 支持PowerShell和CMD环境

## 安全提醒

1. **仅限开发测试使用**，不要在生产环境中使用
2. **不要分享** CA私钥文件
3. **定期更新**证书文件
4. **使用后清理**证书，避免长期信任风险
5. 程序关闭时会自动清理curl环境配置