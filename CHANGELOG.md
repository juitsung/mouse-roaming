# Changelog

本專案遵循 [Keep a Changelog](https://keepachangelog.com/zh-TW/1.1.0/) 的格式規範，並採用 [語意化版本 (Semantic Versioning)](https://semver.org/lang/zh-TW/) 原則進行版本維護。

版本格式：`MAJOR.MINOR.PATCH`
- `MAJOR`：重大不相容更新
- `MINOR`：向後相容的新增功能與重大改進
- `PATCH`：向後相容的錯誤修復與微調

---

## [Unreleased]

### ✨ 新增功能 (Added)
- **連線通知與主機審核機制 (Connection Notification & Approval)**：
  - 新增連線審核模式：預設啟用新連線審核，未經允許前攔截暫緩連線握手。
  - **桌面通知與提示音 (Native Notification & Sound)**：新連線請求、連線成功與斷線時即時發送桌面通知；Windows 呼叫 `MessageBeep`，macOS 呼叫 `afplay Glass.aiff` 發出提示音。
  - **30 秒倒數審核橫幅**：頂部浮現連線核准資訊卡，提供「✔ 允許本次 (Allow Once)」、「🛡 允許並永久信任 (Allow & Remember)」、「✖ 拒絕 (Deny)」，逾時 30 秒自動拒絕並切斷未經授權連線。
  - **信任清單管理**：在設定面板中可隨時檢視與撤銷已永久信任的裝置清單。
- **主機藍牙連線方式 (Bluetooth Host Connection)**：
  - **本機藍牙介面偵測**：自動偵測 Windows / macOS 藍牙網路介面卡與藍牙個人區域網路 (PAN) IP 位址。
  - **已配對主機掃描**：Windows 透過系統註冊表 (`BTHPORT\Parameters\Devices`) 自動讀取已配對的電腦主機名稱、藍牙 MAC 位址與 PAN IP。
  - **一鍵藍牙連線**：在「探索與連線」分頁中新增「藍牙探索與主機連線」子分頁，可對已配對主機進行一鍵連線或輸入藍牙位址連線。
  - **快捷系統藍牙設定**：提供一鍵開啟 Windows / macOS 作業系統內建藍牙設定頁面，方便快速進行主機配對與 PAN 網路共用設定。
  - **傳輸媒介模式切換**：支援區域網路 (LAN)、藍牙 (Bluetooth)、自動雙模 (Auto) 切換。
- **專屬品牌應用程式圖示 (Brand App Icon)**：
  - 導入客製化藍底漸層滑鼠與飛機漫遊視覺識別圖示（抗鋸齒圓角 Squircle 設計）。
  - **Windows 執行檔資源**：透過 `build.rs` 將 multi-resolution `assets/icon.ico` 編譯至執行檔資源中，於檔案總管與工作列完整呈現。
  - **視窗與工作列識別**：主視窗標題列與頂部導覽列即時載入 `assets/icon.png` 紋理。
  - **系統匣／選單列**：系統匣載入專用 32×32 透明背景圖示 `assets/icon_32.png`。
- **縮小時常駐背景 (Minimize to Background / System Tray & Menu Bar)**：
  - **Windows 系統匣常駐 (System Tray)**：點選視窗最小化按鈕 `_`、關閉按鈕 `[X]` 或頂部「🗕 縮小常駐」按鈕時，視窗立即自工作列隱藏，於背景低資源持續運作並常駐系統匣。
  - **macOS 選單列常駐 (Menu Bar)**：點擊最小化或關閉時隱藏主視窗，並於頂部 Menu Bar 保持常駐服務。
  - **快捷喚醒**：支援左鍵單擊／雙擊系統匣圖示或右鍵選單「顯示 / 隱藏主視窗」隨時還原視窗。
  - **安全審核喚醒**：當有新裝置發起連線審核請求時，自動喚起視窗並給予最上層焦點，確保不漏接連線請求。


---

## [0.2.0] - 2026-11-24

### ✨ 新增功能 (Added)
- **視覺化作業系統視窗圖示 (OS Window Icons)**：
  - 取代原本排列畫布上的深色黑塊，根據作業系統類型即時繪製視窗圖示。
  - **Windows**：Windows 11 圓角微軟視窗圖示，含標題列控制項（最小化／最大化／關閉）與中央微軟四格微軟藍標誌。
  - **macOS**：macOS 圓角視窗圖示，含左上方紅黃綠經典三色燈號，並以 4×4 MSAA 抗鋸齒精細繪製蘋果標誌。
  - **Linux / 其他**：終端命令提示視窗與顯示器圖示。
- **裝置標籤資訊擴充**：
  - 各裝置圖示正下方清晰呈現主機名稱（Hostname）與區域網路 IP 位址。
  - 支援自動排除臨時 TCP 連線埠號，顯示乾淨的 IPv4 / IPv6 位址。
- **拖曳定位與螢幕排列互動 (Drag-and-Drop Topology)**：
  - 支援以滑鼠直接按住裝置圖示拖曳至上、下、左、右邊緣插槽。
  - 拖曳過程提供動態虛線連線與發光目標高亮框（磁吸式預覽）。
  - **自動對調 (Auto-Swap)**：當拖曳裝置至已有其他電腦佔用的邊緣時，系統自動將兩台主機的邊緣位置進行互換並即時寫入配置檔。
  - **拖回解除 (Unbind Dock)**：將裝置拖回下方「已連線裝置」停靠區即可解除該裝置的螢幕定位。
- **本機滑鼠漫遊自動隱藏 (Cursor Auto-Hiding)**：
  - 當本機滑鼠移出螢幕邊緣進入遠端主機操作時，本機端的滑鼠游標自動隱藏不見。
  - **Windows**：建立 1×1 全透明空白游標透過 `SetSystemCursor` 替換系統游標群，達成無殘影完整隱藏。
  - **macOS**：呼叫 `CGAssociateMouseAndMouseCursorPosition(false)` 與 `CGDisplayHideCursor` 立即隱藏螢幕游標。
  - 當游標返回本機或遠端斷線時，立即恢復游標顯示（`SPI_SETCURSORS` 與 `CGDisplayShowCursor`）。
- **多語言文件**：
  - 新增繁體中文（[README.md](README.md)）、簡體中文（[README.zh-CN.md](README.zh-CN.md)）與英文（[README.en.md](README.en.md)）詳細使用指南。

### 🔄 變更與最佳化 (Changed)
- 優化主視窗預設尺寸為 820×640，提升螢幕排列畫布與底部連線裝置停靠區的空間佈局。
- 優化拖曳時繪圖更新率，拖曳狀態下啟用 60 FPS 流暢渲染，閒置時自動降至省電輪詢。
- 移除 `Cargo.toml` 中多餘的 workspace package warning 欄位。

### 🐛 錯誤修復 (Fixed)
- 修復 `gui.rs` 中 `status_banner` 在閉包中同時存在可變與不可變借用的編譯錯誤。
- 修復多處浮點數字面值型別回退（float literal fallback）編譯警告。
- 確保在觸發 `Ctrl+Alt+Esc` 硬體緊急逃生鍵、服務終止或行程結束時，本機系統游標保證復原顯示。

---

## [0.1.0] - 2026-11-20

### ✨ 初始發布 (Initial Release)
- **跨平台輸入共享核心**：
  - 支援 Windows 10/11 與 macOS (Apple Silicon & Intel) 無縫鍵鼠共享。
  - Windows 原生 Low-Level Hook (`WH_MOUSE_LL` / `WH_KEYBOARD_LL`) 與 `SendInput`。
  - macOS 原生 `CGEventTap` 與 `CGEventPost`。
- **雙向剪貼簿即時同步**：
  - 支援跨平台 UTF-8 文字剪貼簿同步，具備防回音環迴（Echo Suppression）抑制機制。
- **智慧鍵盤與滾輪映射**：
  - Windows ↔ macOS 自動互換 Ctrl 與 Command (⌘) 鍵。
  - 跨機滾輪方向反轉補償。
- **傳輸與探索安全性**：
  - TLS 1.3 端到端加密傳輸與自簽憑證生成。
  - TOFU（Trust-On-First-Use）SHA-256 憑證指紋信任機制。
  - mDNS 區域網路零設定自動探索廣播。
- **GUI 與系統匣**：
  - 基於 `egui` / `eframe` 的現代暗色系桌面控制面板。
  - 支援最小化至系統匣（System Tray），提供快捷選單與狀態指示。
  - 硬體級緊急逃生快捷鍵（`Ctrl + Alt + Esc`）強制取回本機控制權。
