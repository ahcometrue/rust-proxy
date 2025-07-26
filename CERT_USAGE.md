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

## 证书安装步骤

### macOS系统

1. **安装CA证书**:
   ```bash
   # 将证书添加到系统钥匙串
   sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain certs/ca.crt
   ```

2. **图形界面安装**:
   - 双击 `certs/ca.crt` 文件
   - 选择"系统"钥匙串
   - 双击导入的证书，展开"信任"选项
   - 将"使用此证书时"设置为"始终信任"

### Windows系统

1. **安装CA证书**:
   - 双击 `certs/ca.crt` 文件
   - 选择"安装证书"
   - 存储位置选择"本地计算机"
   - 证书存储选择"受信任的根证书颁发机构"

### Linux系统

1. **Ubuntu/Debian**:
   ```bash
   sudo cp certs/ca.crt /usr/local/share/ca-certificates/study-proxy.crt
   sudo update-ca-certificates
   ```

2. **CentOS/RHEL**:
   ```bash
   sudo cp certs/ca.crt /etc/pki/ca-trust/source/anchors/study-proxy.crt
   sudo update-ca-trust
   ```

## 使用代理

### 1. 启动代理
```bash
cargo run -- --config config.json
```

### 2. 配置系统代理
- **HTTP代理**: `127.0.0.1:8888`
- **HTTPS代理**: `127.0.0.1:8888`

### 3. 浏览器配置
在浏览器中设置代理：
- **Chrome**: 设置 → 系统 → 打开代理设置
- **Firefox**: 设置 → 网络设置 → 手动代理配置

## 配置文件说明

编辑 `config.json` 来自定义代理行为：

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
    "ca_key": "certs/ca.key"
  }
}
```

## 验证证书

### 1. 检查证书信息
```bash
openssl x509 -in certs/ca.crt -text -noout
```

### 2. 测试HTTPS连接
```bash
curl --proxy 127.0.0.1:8888 https://api.github.com
```

## 常见问题

### 1. 证书无效警告
- 确保已正确安装CA证书到系统信任存储
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

## 安全提醒

1. **仅限开发测试使用**，不要在生产环境中使用
2. **不要分享** CA私钥文件
3. **定期更新**证书文件
4. **使用后清理**证书，避免长期信任风险