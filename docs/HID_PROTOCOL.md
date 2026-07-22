# OBOX 蓝牙手柄 HID 协议文档

> VID = `0x0A5C`  PID = `0x4502`(Broadcom BCM20702 蓝牙模组)
> 连接方式: 蓝牙 HID over L2CAP
> 文档用途: 供任意语言接入 ViGEmBus 重写中间件时参考

---

## 1. HID 接口枚举

设备在 Windows 下枚举出 **4 个 HID Collection**(`hid.enumerate` 可见 4 个 path):

| Col | usage_page | usage | 含义 | 用途 |
|-----|-----------|-------|------|------|
| Col01 | 0x0001 (Generic Desktop) | 0x0005 (Gamepad) | **手柄主接口** | 按钮/摇杆/扳机/D-pad |
| Col02 | 0x0001 (Generic Desktop) | 0x0020 (Mouse) | 鼠标(未用) | — |
| Col03 | 0x000C (Consumer) | 0x0001 (Consumer Control) | **媒体键接口** | Back/Menu/Home |
| Col04 | 0x0001 (Generic Desktop) | 0x0006 (Keyboard) | 键盘接口 | PrintScreen 等 |

**关键点:**
- 只读 Col01 和 Col03 即可拿到所有手柄输入
- Col04 被 Windows 键盘栈独占,`hidapi.open_path` 会报 read error,需用全局键盘钩子(如 pynput / Win32 `SetWindowsHookEx WH_KEYBOARD_LL`)读取
- 蓝牙 HID 的 `path` 是 bytes,不要硬编码,运行时用 `enumerate(VENDOR, PRODUCT)` 按 `usage_page` + `usage` 筛选

### 1.1 接口查找伪代码

```python
devices = hid.enumerate(0x0A5C, 0x4502)
gamepad_path  = next(d["path"] for d in devices if d["usage_page"]==0x0001 and d["usage"]==0x0005)
consumer_path = next(d["path"] for d in devices if d["usage_page"]==0x000C and d["usage"]==0x0001)
```

---

## 2. Col01 Gamepad 接口(Report ID 0x07)

### 2.1 报告结构

**Report ID = `0x07`,固定 16 字节:**

| 偏移 | 长度 | 字段 | 说明 |
|------|------|------|------|
| [0]  | 1B   | Report ID | 固定 `0x07` |
| [1-2] | 2B  | Buttons | 16 bit 位图,Little-Endian |
| [3]  | 1B   | Hat + 保留 | 低 4 bit = Hat switch |
| [4-5] | 2B  | LX | 左摇杆 X,Little-Endian |
| [6-7] | 2B  | LY | 左摇杆 Y,Little-Endian |
| [8-9] | 2B  | RX | 右摇杆 X,Little-Endian |
| [10-11] | 2B | RY | 右摇杆 Y,Little-Endian |
| [12-13] | 2B | L2 | 左扳机,Little-Endian |
| [14-15] | 2B | R2 | 右扳机,Little-Endian |

### 2.2 按钮位映射(Buttons,16 bit)

`buttons = data[1] | (data[2] << 8)`,bit 位从 0 起,物理按键编号 = bit 位 + 1。

| Bit | 编号 | 物理键 | Xbox360 映射 |
|-----|------|--------|--------------|
| 0   | B1   | 面键 1 | A |
| 1   | B2   | 面键 2 | B |
| 2   | B3   | —      | (未识别) |
| 3   | B4   | 面键 3 | X |
| 4   | B5   | 面键 4 | Y |
| 5   | B6   | —      | (未识别) |
| 6   | B7   | 左肩键 | LB |
| 7   | B8   | 右肩键 | RB |
| 8-12| B9-B13 | —    | (未识别) |
| 13  | B14  | 左摇杆按下 | L3 (Left Thumb) |
| 14  | B15  | 右摇杆按下 | R3 (Right Thumb) |
| 15  | B16  | —      | (未识别) |

**说明:**
- 共识别出 8 个有效按钮:B1, B2, B4, B5, B7, B8, B14, B15
- 面键 A/B/X/Y 的具体物理对应是推测的,实测如果按 A 显示成 B,直接交换映射表即可
- bit 读取: `is_pressed = (buttons >> bit_pos) & 1`

### 2.3 Hat switch(D-pad,4 bit)

`hat = data[3] & 0x0F`,标准 HID 按钟点排列:

| hat 值 | 方向 | Xbox360 按键 |
|--------|------|--------------|
| 0  | N (上)  | DPAD_UP |
| 1  | NE (右上) | DPAD_UP + DPAD_RIGHT |
| 2  | E (右)  | DPAD_RIGHT |
| 3  | SE (右下) | DPAD_DOWN + DPAD_RIGHT |
| 4  | S (下)  | DPAD_DOWN |
| 5  | SW (左下) | DPAD_DOWN + DPAD_LEFT |
| 6  | W (左)  | DPAD_LEFT |
| 7  | NW (左上) | DPAD_UP + DPAD_LEFT |
| 8 / 15 | 松开 | 无 |

**关键点:** Xbox360 支持斜向同时按下两个方向键,NE/SE/SW/NW 应同时 `press` 两个 `DPAD_*`,不要只映射主方向。

### 2.4 摇杆轴(LX/LY/RX/RY)

- 范围: `0x0000` ~ `0xFFFF` (0 ~ 65535)
- 中心值: `0x8000` (32768)
- 死区建议: ±2000(中心附近抖动)
- **Y 轴方向反转:** HID/DirectInput 中 Y 向下为正,但 XInput (Xbox360) 中 Y 向上为正。转发到 ViGEmBus 时 Y 轴必须取反。

**归一化公式:**
```
delta = raw - 32768
if abs(delta) < 2000:  # 死区
    float_val = 0.0
else:
    float_val = delta / 32768
    if Y轴: float_val = -float_val  # 反转
```

### 2.5 扳机(L2/R2)

- 范围: `0x0000` ~ `0xFFFF` (0 ~ 65535)
- 松开 = 0,按到底 = 65535
- Xbox360 扳机是单字节(0~255),需 `value / 257` 压缩
- 阈值建议: 低于 100 视为 0(消除漂移)

---

## 3. Col03 Consumer 接口(Report ID 0x0A)

### 3.1 报告结构

**Report ID = `0x0A`,固定 7 字节:**

| 偏移 | 长度 | 字段 |
|------|------|------|
| [0]  | 1B   | Report ID = `0x0A` |
| [1-2] | 2B  | Usage slot 0,Little-Endian |
| [3-4] | 2B  | Usage slot 1,Little-Endian |
| [5-6] | 2B  | Usage slot 2,Little-Endian |

### 3.2 Usage 映射

手柄上 3 个非 Gamepad 键走 Consumer 接口,**每次上报当前按下的 Usage 集合**(最多 3 个,松开时槽位为 `0x0000`)。

| Consumer Usage | 物理键 | Xbox360 映射 | Windows 默认行为 |
|----------------|--------|--------------|------------------|
| `0x224` | Back(Android 返回图标) | BACK | 无 |
| `0x040` | Menu(Android 菜单图标) | START | 无 |
| `0x223` | Home(中心键,西瓜键位置) | GUIDE | **打开浏览器**(需注册表禁用) |

### 3.3 状态变化检测(集合运算)

**关键:** 用集合差集检测按下/释放,不要按下标逐个比对 —— 手柄可能把同一个 Usage 放到不同槽位,导致误触发 release+press。

```python
current_usages = {slot0, slot1, slot2} - {0}  # 过滤空槽
pressed  = current_usages - prev_usages
released = prev_usages - current_usages
```

### 3.4 Home 键浏览器行为禁用

Home 键(Usage `0x223`)在 Windows 下默认映射为 `VK_BROWSER_HOME`,按下会打开浏览器。

**禁用方法(注册表):** 删除 `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\AppKey\7` 子键,重启 Explorer 生效。

也可用项目内的 `disable_browser_home.reg` 一键导入。

---

## 4. Col04 Keyboard 接口

### 4.1 状态

- 被 Windows 键盘栈独占,`hidapi.open_path` 会 read error
- 用全局键盘钩子读取(如 pynput 的 `Listener`,或 Win32 `SetWindowsHookEx WH_KEYBOARD_LL`)

### 4.2 已知键

| vk | 物理键 | Xbox360 映射 |
|----|--------|--------------|
| `0x2C` | PrintScreen | (无对应 Xbox360 键,可不映射) |

手柄上只有 1 个键走 Keyboard 接口(实测),且当前未映射到 Xbox360 任何按钮。

---

## 5. 转发到 ViGEmBus(Xbox360)

### 5.1 架构

```
Col01 Gamepad (HID)  ─┐
Col03 Consumer (HID) ──┼──> ControllerState (线程安全) ──> OutputThread ──> ViGEmBus Xbox360
Keyboard (全局钩子)  ─┘
```

### 5.2 关键实现点

1. **三路输入合并:** Col01 / Col03 / Keyboard 各开一个线程,写入同一个线程安全的 `ControllerState`(锁保护)
2. **dirty 驱动输出:** 仅当状态变化时调用 `gamepad.update()`,静止时跳过省 CPU
3. **断线重连:** HID `read` 抛异常后,3 秒重新 `enumerate` 查找 path,自动恢复
4. **全量更新:** 每次 update 前 `gamepad.reset()`,再按当前状态重新 `press_button` / `set_joystick` / `set_trigger`

### 5.3 vgamepad API 速查(Python)

```python
import vgamepad as vg

gamepad = vg.VX360Gamepad()

# 按钮
gamepad.press_button(vg.XUSB_BUTTON.XUSB_GAMEPAD_A)
gamepad.release_button(vg.XUSB_BUTTON.XUSB_GAMEPAD_A)

# 摇杆 (float: -1.0 ~ 1.0)
gamepad.left_joystick_float(x, y)   # y 向上为正
gamepad.right_joystick_float(x, y)

# 扳机 (0 ~ 255)
gamepad.left_trigger(value)
gamepad.right_trigger(value)

# 重置 + 提交
gamepad.reset()
gamepad.update()
```

### 5.4 Xbox360 按钮枚举

```
XUSB_GAMEPAD_A               = 0x1000
XUSB_GAMEPAD_B               = 0x2000
XUSB_GAMEPAD_X               = 0x4000
XUSB_GAMEPAD_Y               = 0x8000
XUSB_GAMEPAD_LEFT_SHOULDER   = 0x0100   (LB)
XUSB_GAMEPAD_RIGHT_SHOULDER  = 0x0200   (RB)
XUSB_GAMEPAD_LEFT_THUMB      = 0x0040   (L3)
XUSB_GAMEPAD_RIGHT_THUMB     = 0x0080   (R3)
XUSB_GAMEPAD_START           = 0x0010
XUSB_GAMEPAD_BACK            = 0x0020
XUSB_GAMEPAD_GUIDE           = 0x0004   (西瓜键)
XUSB_GAMEPAD_DPAD_UP         = 0x0001
XUSB_GAMEPAD_DPAD_DOWN       = 0x0002
XUSB_GAMEPAD_DPAD_LEFT       = 0x0004
XUSB_GAMEPAD_DPAD_RIGHT      = 0x0008
```

> 注意: `XUSB_GAMEPAD_GUIDE` 和 `XUSB_GAMEPAD_DPAD_LEFT` 的数值相同(0x0004),但它们在不同的位字段中(按钮 vs D-pad),vgamepad 内部会正确处理。在 C/C++ ViGEmBus API 中,它们是 `XUSB_REPORT.wButtons` 的不同 bit,不冲突。

---

## 6. 校准参数

| 参数 | 值 | 说明 |
|------|----|------|
| STICK_CENTER | 32768 (0x8000) | 摇杆中心 |
| STICK_DEADZONE | 2000 | 死区,中心 ±2000 内视为 0 |
| TRIGGER_THRESHOLD | 100 | 扳机阈值,低于此值视为 0 |
| OUTPUT_INTERVAL | 5ms (200Hz) | 输出线程刷新间隔上限(实际 dirty 驱动) |

---

## 7. 已知问题与坑

1. **Home 键弹浏览器** — 见 §3.4,注册表禁用
2. **Y 轴方向** — HID 与 XInput 相反,转发时必须取反,见 §2.4
3. **D-pad 斜向** — Xbox360 支持同时按两个方向,见 §2.3
4. **Keyboard 接口独占** — 见 §4.1,必须用全局钩子
5. **Consumer Usage 槽位顺序** — 见 §3.3,必须用集合运算
6. **hidapi 包冲突** — Python 下装 `hidapi` 包,不要同时装 `hid`,两者会冲突
7. **path 是 bytes** — 蓝牙 HID 的 `path` 含字符串,不要硬编码,运行时枚举
8. **ViGEmBus 驱动** — 必须先装 ViGEmBus Setup,否则 `VX360Gamepad()` 构造失败

---

## 8. 快速验证清单

重写后按此顺序验证:

- [ ] 枚举到 4 个 HID 接口,筛出 Col01 + Col03
- [ ] 读到 Report ID 0x07 (16B) 和 0x0A (7B)
- [ ] 按 8 个 Gamepad 键,对应 bit 置位
- [ ] 推 D-pad 4 个正方向,hat = 0/2/4/6
- [ ] 推 D-pad 4 个斜方向,hat = 1/3/5/7,且 Xbox360 端同时亮两个方向
- [ ] 推摇杆到四周极限,值接近 0 或 65535,Y 轴方向正确(向上 = 正)
- [ ] 按 L2/R2 扳机,值 0 → 65535
- [ ] 按 Back/Menu/Home,对应 Consumer Usage 0x224/0x040/0x223
- [ ] Home 键不弹浏览器(注册表已禁用)
- [ ] 拔掉手柄后重连,中间件 3 秒内自动恢复
- [ ] 手柄静止时,CPU 占用接近 0(dirty 驱动生效)
