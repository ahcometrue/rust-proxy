# 证书自动安装文档

本文档详细说明了Study Proxy的证书自动安装功能，该功能允许程序自动将生成的CA证书安装到系统信任存储，使curl等工具无需手动指定证书即可使用代理。

## 功能概述

Study Proxy现在支持以下证书管理功能：

1. **自动证书安装**：程序启动时自动将CA证书安装到系统信任存储
2. **跨平台支持**：支持Windows、macOS和Linux系统
3. **自动清理**：程序退出时自动从系统信任存储中移除证书
4. **可配置选项**：通过配置文件灵活启用/禁用此功能

## 配置说明

在`config.json`中添加证书安装配置：

```json
{
  "certificates": {
    "ca_cert": "certs/ca.crt",
    "ca_key": "certs/ca.key",
    "auto_install": true,
    "auto_uninstall": true,
    "name": "study-proxy"
  }
}
```

### 配置选项

- `ca_cert`: CA证书文件路径（默认："certs/ca.crt"）
- `ca_key`: CA私钥文件路径（默认："certs/ca.key"）
- `auto_install`: 是否自动安装证书到系统信任存储（默认：true）
- `auto_uninstall`: 程序关闭时是否自动卸载证书（默认：true）
- `name`: 证书名称，用于在系统信任存储中标识证书（默认："study-proxy"）

## 平台支持详情

### macOS
- **证书存储**: 系统钥匙串（System.keychain）
- **命令**: 使用`security`命令
- **权限**: 需要管理员权限（sudo）
- **证书名称**: "Study Proxy CA"

### Windows
- **证书存储**: 受信任的根证书颁发机构
- **命令**: 使用`certutil`命令
- **权限**: 需要管理员权限
- **证书名称**: "Study Proxy CA"

### Linux
- **证书存储**: /usr/local/share/ca-certificates/
- **命令**: 使用`update-ca-certificates`
- **权限**: 需要管理员权限（sudo）
- **证书名称**: study-proxy-ca.crt

## 使用方法

### 1. 启用证书自动安装（默认启用）
```json
{
  "certificates": {
    "ca_cert": "certs/ca.crt",
    "ca_key": "certs/ca.key",
    "auto_install": true
  }
}
```

### 2. 禁用证书自动安装
```json
{
  "certificates": {
    "ca_cert": "certs/ca.crt",
    "ca_key": "certs/ca.key",
    "auto_install": false
  }
}
```

### 3. 程序启动日志
当证书自动安装启用时，程序启动会显示：
```
[2024-XX-XX] Installing CA certificate to system trust store...
[2024-XX-XX] CA certificate successfully installed to macOS system keychain
```

### 4. 程序退出日志
程序关闭时会自动清理证书：
```
[2024-XX-XX] Removing CA certificate from system trust store...
[2024-XX-XX] CA certificate successfully removed from system trust store
```

## 使用curl测试

启用证书自动安装后，使用curl无需手动指定证书：

```bash
# 直接访问HTTPS网站，无需--cacert参数
curl -x http://127.0.0.1:8888 https://example.com

# 之前需要这样：
# curl --cacert certs/ca.crt -x http://127.0.0.1:8888 https://example.com
```

## 故障排除

### macOS权限问题
如果遇到权限错误，请使用sudo运行：
```bash
sudo cargo run
```

### 证书已存在
如果证书已存在于系统信任存储中，程序会跳过安装：
```
[2024-XX-XX] CA certificate already installed on macOS
```

### Linux证书更新失败
确保update-ca-certificates命令可用：
```bash
sudo apt-get install ca-certificates  # Ubuntu/Debian
sudo yum install ca-certificates      # CentOS/RHEL
```

### Windows证书管理
确保certutil命令可用（Windows自带）：
```cmd
# 以管理员身份运行
certutil -addstore -f Root certs/ca.crt
```

## 手动安装证书

如果不想使用自动安装，可以手动安装证书：

### macOS
```bash
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain certs/ca.crt
```

### Windows
```cmd
certutil -addstore -f Root certs/ca.crt
```

### Linux
```bash
sudo cp certs/ca.crt /usr/local/share/ca-certificates/study-proxy-ca.crt
sudo update-ca-certificates
```

## 注意事项

1. **权限要求**: 证书安装需要管理员权限，请使用sudo运行程序
2. **证书有效期**: 生成的CA证书有效期为10年（2024-2034）
3. **证书清理**: 程序退出时会自动清理安装的证书
4. **系统兼容性**: 不同操作系统可能有细微差异，请根据平台调整使用
5. **浏览器信任**: 证书安装后，浏览器也会自动信任该CA证书

## 验证证书安装

可以通过以下方式验证证书是否正确安装：

### macOS
```bash
security find-certificate -c "Study Proxy CA" /Library/Keychains/System.keychain
```

### Windows
```cmd
certutil -store Root "Study Proxy CA"
```

### Linux
```bash
ls /usr/local/share/ca-certificates/ | grep study-proxy
openssl x509 -in /usr/local/share/ca-certificates/study-proxy-ca.crt -text -noout
```