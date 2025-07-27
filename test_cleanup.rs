use std::env;
use std::fs;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let home_dir = env::var("HOME")?;
    
    let shell_config = if shell.contains("zsh") {
        PathBuf::from(&home_dir).join(".zshrc")
    } else if shell.contains("bash") {
        PathBuf::from(&home_dir).join(".bashrc")
    } else {
        PathBuf::from(&home_dir).join(".profile")
    };
    
    println!("Testing cleanup for: {:?}", shell_config);
    
    // 读取当前内容
    let content = fs::read_to_string(&shell_config)?;
    println!("Original content:\n{}\n", content);
    
    // 应用清理逻辑
    let lines: Vec<&str> = content.lines().collect();
    let mut new_lines = Vec::new();
    let mut skip_mode = false;
    
    for line in lines {
        let trimmed = line.trim();
        
        if trimmed == "# Study Proxy Auto Configuration" {
            skip_mode = true;
            continue;
        }
        
        if skip_mode {
            if trimmed.starts_with("export HTTPS_PROXY=") ||
               trimmed.starts_with("export HTTP_PROXY=") ||
               trimmed.starts_with("export CURL_CA_BUNDLE=") {
                continue;
            }
            
            if !trimmed.is_empty() && !trimmed.starts_with("export ") {
                skip_mode = false;
                new_lines.push(line);
            }
            continue;
        }
        
        new_lines.push(line);
    }
    
    // 清理空行
    let mut final_lines: Vec<&str> = Vec::new();
    let mut prev_empty = false;
    
    for line in new_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_empty {
                final_lines.push(line);
                prev_empty = true;
            }
        } else {
            final_lines.push(line);
            prev_empty = false;
        }
    }
    
    while let Some(last) = final_lines.last() {
        if last.trim().is_empty() {
            final_lines.pop();
        } else {
            break;
        }
    }
    
    let new_content = final_lines.join("\n");
    println!("New content:\n{}\n", new_content);
    
    fs::write(&shell_config, new_content)?;
    println!("Cleanup completed successfully");
    
    Ok(())
}