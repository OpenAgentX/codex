 Windows 下从 0 到连通步骤                                          
                                                                     
                                                                     
• 下面按 Windows 从 0 到连通给你一套最短路径。先说前提：这个链路能把 
  你写的 Rust backend 和 html+js controller 跑起来，但 Codex 端仍然要
  求 ChatGPT 登录态，这是当前远控实现的硬限制，不是 demo 限制。      

  1. 准备环境                                                        
                                                                     
  安装并确认这些工具可用：                                           
                                                                     
  rustc --version                                                    
  cargo --version                                                    
  git --version                                                      
                                                                     
  进入仓库：                                                         
                                                                     
  cd D:\agentx\codex\codex-rs                                        
                                                                     
  我加的两个文件在这里：                                             
                                                                     
  - codex-rs/app-server-transport/examples/remote_control_backend.rs 
  - codex-rs/app-server-transport/examples/remote_controller.html    
                                                                     
  2. 启动 Rust Remote Backend                                        
                                                                     
  在 codex-rs 目录执行：                                             
                                                                     
  cargo run -p codex-app-server-transport --example                  
remote_control_backend                                               
                                                                     
  正常会监听：                                                       
                                                                     
  http://127.0.0.1:8787                                              
                                                                     
  它提供：                                                           
                                                                     
  - POST /backend-api/wham/remote/control/server/enroll
  - GET /backend-api/wham/remote/control/server                      
  - GET /controller/ws/:environment_id                               
  - GET /environments                                                
  - GET /                                                            
                                                                     
  3. 配置 Codex 指向本地 backend                                     
                                                                     
  把 Codex 的 chatgpt_base_url 改到本地。                            
                                                                     
  通常用户配置文件在：                                               
                                                                     
  C:\Users\Administrator\.codex\config.toml                          
                                                                     
  至少加入这一行：                                                   
                                                                     
  chatgpt_base_url = "http://127.0.0.1:8787/backend-api/"            
                                                                     
  如果文件不存在，就新建。                                           
                                                                     
  4. 先完成 Codex 登录                                               
                                                                     
  这是必须的。你需要让 Codex 处于 ChatGPT 登录态。否则远控连接阶段会 
  被拒绝，因为当前实现明确不支持 API key 直接做 remote control。     
                                                                     
  如果你没登录，先执行你平时用的 Codex 登录流程。                    
                                                                     
  5. 启动远控 app-server                                             
                                                                     
  二选一。                                                           
                                                                     
  临时启动：                                                         
                                                                     
  codex remote-control start                                         
                                                                     
  或者持久化 daemon 方式：                                           
                                                                     
  codex app-server daemon bootstrap --remote-control                 
                                                                     
  如果 daemon 已存在，也可以：                                       
                                                                     
  codex app-server daemon enable-remote-control                      
                                                                     
  6. 打开浏览器控制端                                                
                                                                     
  浏览器访问：                                                       
                                                                     
  http://127.0.0.1:8787/                                             
                                                                     
  页面里先点 List Environments。                                     
                                                                     
  如果第 5 步成功，backend 会收到 Codex 的 enrollment，页面会看到一个
  environmentId，例如：                                              
                                                                     
  env_xxx                                                            
                                                                     
  把它填到页面里的 Environment ID，然后点 Connect Controller。       
                                                                     
  7. 建立 JSON-RPC 会话
                                                                     
  连接 controller 后，按这个顺序发：                                 
                                                                     
  1. Send initialize                                                 
  2. Send initialized                                                
                                                                     
  然后你就可以在 Raw JSON-RPC Message 里发请求，比如：               
                                                                     
  {                                                                  
    "jsonrpc": "2.0",                                                
    "id": 2,                                                         
    "method": "app/list",                                            
    "params": {}                                                     
  }                                                                  
                                                                     
  点 Send RPC。                                                      
                                                                     
  Codex 返回的响应、通知、远控 envelope 都会出现在页面右侧日志里。   
                                                                     
  8. 连通成功的判断                                                  
                                                                     
  满足这几条就算通了：                                               
                                                                     
  - backend 进程还在运行                                             
  - http://127.0.0.1:8787/environments 能看到至少一个环境            
  - 页面状态里 codexConnected=true                                   
  - initialize 后能收到 Codex 的 JSON-RPC response                   
  - 发 app/list、model/list 之类请求能收到结果                       
                                                                     
  9. 最常见的失败点                                                  
                                                                     
  List Environments 为空                                             
  说明 Codex 还没连上你的 Rust backend。优先检查：                   
                                                                     
  - config.toml 里的 chatgpt_base_url                                
  - backend 是否真在 127.0.0.1:8787                                  
  - 你是否执行了 codex remote-control start                          
                                                                     
  提示需要 ChatGPT authentication                                    
  这是预期限制。当前远控实现要求 ChatGPT 登录态，不支持纯 API key。  

  浏览器连上了，但 codexConnected=false
  说明 controller 连上 backend 了，但 Codex websocket 没连上         
  backend。                                                          
                                                                     
  Windows 上 cargo test 或某些构建失败
  这个仓库在 Windows 下可能会卡在 v8 相关依赖和 symlink 权限，不影响 
  你先跑这个 example backend；cargo run -p codex-app-server-transport
  --example remote_control_backend 才是关键路径。                    
                                                                     
  10. 建议的最小联通验证顺序                                         
                                                                     
  cd D:\agentx\codex\codex-rs                                        
  cargo run -p codex-app-server-transport --example
remote_control_backend                                               
                                                                     
  然后：

  1. 改 C:\Users\Administrator\.codex\config.toml                    
  2. 确保 Codex 已登录 ChatGPT                                       
  3. 执行 codex remote-control start                                 
  4. 打开 http://127.0.0.1:8787/                                     
  5. List Environments                                               
  6. Connect Controller                                              
  7. Send initialize                                                 
  8. Send initialized                                                
  9. 发一个 app/list                                                 
                                                                     
  如果你要，我下一条可以直接给你一份可直接粘贴的 Windows config.toml 
  最小配置，以及一组适合验证的 JSON-RPC 请求样例。