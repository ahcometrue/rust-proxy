# 跨平台系统代理自动配置

本项目支持在Windows、macOS和Linux上自动配置系统代理设置。

## 功能特性

- **跨平台支持**: Windows、macOS、Linux
- **自动配置**: 程序启动时自动设置系统代理
- **自动清理**: 程序关闭时自动恢复原始代理设置
- **信号处理**: 支持Ctrl+C中断时的代理清理
- **配置灵活**: 可通过配置文件启用/禁用此功能

## 配置说明

在`config.json`中添加系统代理配置：

```json
{
  "proxy": {
    "host": "127.0.0.1",
    "port": 8888
  },
  "system_proxy": {
    "enabled": true
  }
}
```

### 配置选项

- `enabled`: 是否启用系统代理功能（默认：true）

## 平台支持详情

### macOS
- **网络接口**: 自动配置Wi-Fi代理
- **代理类型**: HTTP和HTTPS代理
- **命令**: 使用`networksetup`命令
- **权限**: 需要管理员权限

### Windows
- **注册表**: 修改用户注册表设置
- **代理类型**: HTTP和HTTPS代理
- **注册表路径**: `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Internet Settings`

### Linux
- **环境变量**: 设置http_proxy和https_proxy
- **GNOME**: 支持GNOME桌面环境的gsettings配置
- **代理类型**: HTTP和HTTPS代理

## 使用方法

### 1. 启用系统代理（默认启用）
```json
{
  "system_proxy": {
    "enabled": true
  }
}
```

### 2. 禁用系统代理
```json
{
  "system_proxy": {
    "enabled": false
  }
}
```

### 3. 手动控制
程序启动时会显示代理配置状态：
```
[2024-XX-XX] System proxy configured successfully
```

程序关闭时会自动清理：
```
[2024-XX-XX] Cleaning up system proxy settings...
[2024-XX-XX] System proxy settings restored
```

## 故障排除

### macOS权限问题
如果遇到权限错误，请使用sudo运行：
```bash
sudo cargo run
```

### Windows注册表访问
确保用户有权限修改注册表，通常不需要额外权限。

### Linux环境变量
某些应用程序可能不会自动使用系统代理环境变量，需要手动配置。

## 手动配置示例

如果不想使用自动配置，可以手动设置代理：

### macOS
```bash
# 设置代理
networksetup -setwebproxy Wi-Fi 127.0.0.1 8888
networksetup -setsecurewebproxy Wi-Fi 127.0.0.1 8888

# 禁用代理
networksetup -setwebproxystate Wi-Fi off
networksetup -setsecurewebproxystate Wi-Fi off
```

### Windows
```cmd
# 设置代理
reg add "HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Internet Settings" /v ProxyEnable /t REG_DWORD /d 1 /f
reg add "HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Internet Settings" /v ProxyServer /t REG_SZ /d "127.0.0.1:8888" /f

# 禁用代理
reg add "HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Internet Settings" /v ProxyEnable /t REG_DWORD /d 0 /f
```

### Linux
```bash
# 临时设置
export http_proxy=http://127.0.0.1:8888
export https_proxy=http://127.0.0.1:8888

# GNOME桌面
 gsettings set org.gnome.system.proxy mode 'manual'
gsettings set org.gnome.system.proxy.http host '127.0.0.1'
gsettings set org.gnome.system.proxy.http port 8888
```

## 注意事项

1. **权限**: 某些平台可能需要管理员权限
2. **网络接口**: macOS默认配置Wi-Fi接口，如需配置其他接口请修改代码
3. **持久性**: 代理设置在程序关闭后会自动恢复
4. **兼容性**: 不同Linux发行版可能有差异，环境变量方式最通用