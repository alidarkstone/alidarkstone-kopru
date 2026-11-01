// Köprü giriş yardımcısı (Windows). Kaynağı tamamen bu dosyadır; kurulumda bilgisayarınızda
// Windows'un kendi C# derleyicisiyle (csc.exe) derlenir. Hazır .exe dağıtılmaz.
//
// Ne yapar:
//   - Fare/klavye paylaşımı açıkken imlecin ekran kenarına gelip gelmediğini izler (yalnızca fare kancası).
//   - Kenara gelince ve uygulama "yakala" derse fare ve klavye olaylarını yerelde engeller,
//     stdout'a JSON satırları olarak yazar (uygulama bunları şifreli kanalla diğer bilgisayara iletir).
//   - Diğer bilgisayardan gelen olayları SendInput ile bu bilgisayarda uygular.
// Ne YAPMAZ: ağa bağlanmaz, dosya yazmaz, kayıt defterine dokunmaz, başka program çalıştırmaz.
// Klavye kancası yalnızca imleç diğer bilgisayardayken kurulur ve geri dönünce kaldırılır.
// Acil çıkış: imleç diğer bilgisayardayken Scroll Lock tuşu kontrolü hemen geri alır.
// Uygulama kapanırsa (stdin kapanır) yardımcı da kendini kapatır.

using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using System.Windows.Forms;

static class KopruInput
{
    // ---------------- Win32 ----------------
    const int WH_KEYBOARD_LL = 13, WH_MOUSE_LL = 14;
    const int WM_MOUSEMOVE = 0x200, WM_LBUTTONDOWN = 0x201, WM_LBUTTONUP = 0x202, WM_RBUTTONDOWN = 0x204, WM_RBUTTONUP = 0x205,
              WM_MBUTTONDOWN = 0x207, WM_MBUTTONUP = 0x208, WM_MOUSEWHEEL = 0x20A, WM_XBUTTONDOWN = 0x20B, WM_XBUTTONUP = 0x20C,
              WM_MOUSEHWHEEL = 0x20E;
    const int WM_KEYDOWN = 0x100, WM_KEYUP = 0x101, WM_SYSKEYDOWN = 0x104, WM_SYSKEYUP = 0x105;
    const uint LLKHF_EXTENDED = 0x01, LLKHF_INJECTED = 0x10, LLMHF_INJECTED = 0x01;
    const uint INPUT_MOUSE = 0, INPUT_KEYBOARD = 1;
    const uint MOUSEEVENTF_MOVE = 0x1, MOUSEEVENTF_LEFTDOWN = 0x2, MOUSEEVENTF_LEFTUP = 0x4, MOUSEEVENTF_RIGHTDOWN = 0x8,
               MOUSEEVENTF_RIGHTUP = 0x10, MOUSEEVENTF_MIDDLEDOWN = 0x20, MOUSEEVENTF_MIDDLEUP = 0x40, MOUSEEVENTF_XDOWN = 0x80,
               MOUSEEVENTF_XUP = 0x100, MOUSEEVENTF_WHEEL = 0x800, MOUSEEVENTF_HWHEEL = 0x1000, MOUSEEVENTF_VIRTUALDESK = 0x4000,
               MOUSEEVENTF_ABSOLUTE = 0x8000;
    const uint KEYEVENTF_EXTENDEDKEY = 0x1, KEYEVENTF_KEYUP = 0x2;
    const int VK_SCROLL = 0x91;
    const int SM_XVIRTUALSCREEN = 76, SM_YVIRTUALSCREEN = 77, SM_CXVIRTUALSCREEN = 78, SM_CYVIRTUALSCREEN = 79;

    [StructLayout(LayoutKind.Sequential)] struct POINT { public int x, y; }
    [StructLayout(LayoutKind.Sequential)] struct MSLLHOOKSTRUCT { public POINT pt; public uint mouseData, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct KBDLLHOOKSTRUCT { public uint vkCode, scanCode, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public INPUTUNION u; }

    delegate IntPtr HookProc(int code, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", SetLastError = true)] static extern IntPtr SetWindowsHookEx(int id, HookProc fn, IntPtr mod, uint thread);
    [DllImport("user32.dll")] static extern bool UnhookWindowsHookEx(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr CallNextHookEx(IntPtr h, int code, IntPtr w, IntPtr l);
    [DllImport("kernel32.dll")] static extern IntPtr GetModuleHandle(string name);
    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr v);

    // ---------------- durum ----------------
    static Control ui;                       // kancaların kurulduğu, mesaj döngüsü olan iş parçacığı
    static readonly object outLock = new object();
    static HookProc mouseProc = MouseHook, keyProc = KeyHook; // GC toplamasın diye alanda tutulur
    static IntPtr mouseHook = IntPtr.Zero, keyHook = IntPtr.Zero;
    static string watchEdge = null;          // yerel: diğer bilgisayar bu kenarda (left/right/top/bottom)
    static string mode = "idle";             // idle | capturing | controlled
    static string returnEdge = null;         // kontrol edilirken: bu kenardan çıkınca kontrol geri döner
    static POINT anchor;                     // yakalarken imlecin sabit durduğu nokta
    static long cooldownUntil = 0;
    static double vx, vy;                    // kontrol edilirken sanal imleç (alt piksel birikimi için)
    static readonly HashSet<string> downKeys = new HashSet<string>();
    static readonly HashSet<string> downButtons = new HashSet<string>();
    static readonly JavaScriptSerializer json = new JavaScriptSerializer();

    [STAThread]
    static void Main()
    {
        try { if (!SetProcessDpiAwarenessContext(new IntPtr(-4))) SetProcessDPIAware(); }
        catch (EntryPointNotFoundException) { SetProcessDPIAware(); }
        Console.OutputEncoding = new UTF8Encoding(false);

        ui = new Control();
        ui.CreateControl();
        var h = ui.Handle; // pencere tutamacını bu iş parçacığında oluştur

        var reader = new Thread(ReadLoop) { IsBackground = true };
        reader.Start();
        Emit("{\"ev\":\"ready\"}");
        Application.Run();
        Cleanup();
    }

    static void ReadLoop()
    {
        var stdin = new StreamReader(Console.OpenStandardInput(), new UTF8Encoding(false));
        string line;
        while ((line = stdin.ReadLine()) != null)
        {
            Dictionary<string, object> m;
            try { m = json.Deserialize<Dictionary<string, object>>(line); } catch { continue; }
            if (m == null) continue;
            var msg = m;
            try { ui.BeginInvoke((MethodInvoker)(() => Handle(msg))); } catch { break; }
        }
        // Uygulama kapandı: her şeyi bırak ve çık.
        try { ui.BeginInvoke((MethodInvoker)(() => { Cleanup(); Application.ExitThread(); })); } catch { }
        Thread.Sleep(500);
        Environment.Exit(0);
    }

    static void Emit(string s) { lock (outLock) { Console.Out.Write(s + "\n"); Console.Out.Flush(); } }

    static string Str(Dictionary<string, object> m, string k) { object v; return m.TryGetValue(k, out v) && v != null ? v.ToString() : null; }
    static double Num(Dictionary<string, object> m, string k) { object v; return m.TryGetValue(k, out v) && v != null ? Convert.ToDouble(v, System.Globalization.CultureInfo.InvariantCulture) : 0; }
    static bool Bool(Dictionary<string, object> m, string k) { object v; return m.TryGetValue(k, out v) && v is bool && (bool)v; }

    static bool ValidEdge(string e) { return e == "left" || e == "right" || e == "top" || e == "bottom"; }

    static void Handle(Dictionary<string, object> m)
    {
        switch (Str(m, "cmd"))
        {
            case "edge":
                var e = Str(m, "edge");
                watchEdge = ValidEdge(e) ? e : null;
                UpdateMouseHook();
                break;
            case "capture":
                if (Bool(m, "on")) StartCapture(); else StopCapture(Num(m, "pos"), m.ContainsKey("pos"));
                break;
            case "enter": // diğer bilgisayardan kontrol başlıyor; imleci giriş kenarına koy
                var en = Str(m, "edge");
                if (!ValidEdge(en)) break;
                if (mode == "capturing") StopCapture(0.5, false);
                mode = "controlled";
                returnEdge = en;
                PlaceAtEdge(en, Clamp01(Num(m, "pos")), 2);
                POINT p; GetCursorPos(out p); vx = p.x; vy = p.y;
                break;
            case "leave": // kontrol bitti
                if (mode == "controlled") { ReleaseAll(); mode = "idle"; returnEdge = null; cooldownUntil = Environment.TickCount + 300; }
                break;
            case "move": if (mode == "controlled") InjectMove(Num(m, "dx"), Num(m, "dy")); break;
            case "button": if (mode == "controlled") InjectButton(Str(m, "b"), Bool(m, "down")); break;
            case "wheel": if (mode == "controlled") InjectWheel((int)Num(m, "d"), Bool(m, "h")); break;
            case "key": if (mode == "controlled") InjectKey((int)Num(m, "vk"), (int)Num(m, "sc"), Bool(m, "ext"), Bool(m, "down")); break;
            case "releaseAll": ReleaseAll(); break;
            case "quit": Cleanup(); Application.ExitThread(); break;
        }
    }

    // ---------------- ekran geometrisi ----------------
    static int VL { get { return GetSystemMetrics(SM_XVIRTUALSCREEN); } }
    static int VT { get { return GetSystemMetrics(SM_YVIRTUALSCREEN); } }
    static int VW { get { return GetSystemMetrics(SM_CXVIRTUALSCREEN); } }
    static int VH { get { return GetSystemMetrics(SM_CYVIRTUALSCREEN); } }
    static double Clamp01(double v) { return v < 0 ? 0 : v > 1 ? 1 : v; }

    static void PlaceAtEdge(string edge, double pos, int inset)
    {
        int x, y;
        if (edge == "left" || edge == "right")
        {
            y = VT + (int)Math.Round(pos * (VH - 1));
            x = edge == "left" ? VL + inset : VL + VW - 1 - inset;
        }
        else
        {
            x = VL + (int)Math.Round(pos * (VW - 1));
            y = edge == "top" ? VT + inset : VT + VH - 1 - inset;
        }
        SetCursorPos(x, y);
    }

    static double PosAlong(string edge, POINT p)
    {
        return edge == "left" || edge == "right" ? Clamp01((p.y - VT) / (double)Math.Max(1, VH - 1)) : Clamp01((p.x - VL) / (double)Math.Max(1, VW - 1));
    }

    static bool AtEdge(string edge, POINT p)
    {
        switch (edge)
        {
            case "left": return p.x <= VL;
            case "right": return p.x >= VL + VW - 1;
            case "top": return p.y <= VT;
            case "bottom": return p.y >= VT + VH - 1;
        }
        return false;
    }

    // ---------------- kancalar ----------------
    static void UpdateMouseHook()
    {
        bool need = watchEdge != null || mode == "capturing";
        if (need && mouseHook == IntPtr.Zero) mouseHook = SetWindowsHookEx(WH_MOUSE_LL, mouseProc, GetModuleHandle(null), 0);
        else if (!need && mouseHook != IntPtr.Zero) { UnhookWindowsHookEx(mouseHook); mouseHook = IntPtr.Zero; }
    }

    static void StartCapture()
    {
        if (mode != "idle" || watchEdge == null) return;
        mode = "capturing";
        UpdateMouseHook();
        if (keyHook == IntPtr.Zero) keyHook = SetWindowsHookEx(WH_KEYBOARD_LL, keyProc, GetModuleHandle(null), 0);
        // İmleci ana ekranın ortasına al: oradan her yöne hareket ölçülebilir.
        var b = Screen.PrimaryScreen.Bounds;
        anchor.x = b.Left + b.Width / 2; anchor.y = b.Top + b.Height / 2;
        SetCursorPos(anchor.x, anchor.y);
    }

    static void StopCapture(double pos, bool place)
    {
        if (mode != "capturing") return;
        mode = "idle";
        if (keyHook != IntPtr.Zero) { UnhookWindowsHookEx(keyHook); keyHook = IntPtr.Zero; }
        downKeys.Clear(); downButtons.Clear();
        if (watchEdge != null) PlaceAtEdge(watchEdge, place ? Clamp01(pos) : 0.5, 3);
        cooldownUntil = Environment.TickCount + 300;
        UpdateMouseHook();
    }

    static IntPtr MouseHook(int code, IntPtr w, IntPtr l)
    {
        if (code < 0) return CallNextHookEx(mouseHook, code, w, l);
        var s = (MSLLHOOKSTRUCT)Marshal.PtrToStructure(l, typeof(MSLLHOOKSTRUCT));
        int msg = w.ToInt32();
        if ((s.flags & LLMHF_INJECTED) != 0) return CallNextHookEx(mouseHook, code, w, l);

        if (mode == "capturing")
        {
            switch (msg)
            {
                case WM_MOUSEMOVE:
                    int dx = s.pt.x - anchor.x, dy = s.pt.y - anchor.y;
                    if (dx != 0 || dy != 0) Emit("{\"ev\":\"move\",\"dx\":" + dx + ",\"dy\":" + dy + "}");
                    break;
                case WM_LBUTTONDOWN: Button("left", true); break;
                case WM_LBUTTONUP: Button("left", false); break;
                case WM_RBUTTONDOWN: Button("right", true); break;
                case WM_RBUTTONUP: Button("right", false); break;
                case WM_MBUTTONDOWN: Button("middle", true); break;
                case WM_MBUTTONUP: Button("middle", false); break;
                case WM_XBUTTONDOWN: Button(((s.mouseData >> 16) & 0xFFFF) == 1 ? "x1" : "x2", true); break;
                case WM_XBUTTONUP: Button(((s.mouseData >> 16) & 0xFFFF) == 1 ? "x1" : "x2", false); break;
                case WM_MOUSEWHEEL:
                case WM_MOUSEHWHEEL:
                    short d = (short)((s.mouseData >> 16) & 0xFFFF);
                    Emit("{\"ev\":\"wheel\",\"d\":" + d + ",\"h\":" + (msg == WM_MOUSEHWHEEL ? "true" : "false") + "}");
                    break;
            }
            return new IntPtr(1); // olayı yerelde yut
        }

        if (mode == "idle" && msg == WM_MOUSEMOVE && watchEdge != null && Environment.TickCount > cooldownUntil
            && AtEdge(watchEdge, s.pt) && downButtons.Count == 0 && Control.MouseButtons == MouseButtons.None)
        {
            cooldownUntil = Environment.TickCount + 500; // uygulama yanıt verene kadar tekrar tetikleme
            Emit("{\"ev\":\"edge\",\"pos\":" + PosAlong(watchEdge, s.pt).ToString("0.0000", System.Globalization.CultureInfo.InvariantCulture) + "}");
        }
        return CallNextHookEx(mouseHook, code, w, l);
    }

    static void Button(string b, bool down)
    {
        if (down) downButtons.Add(b); else downButtons.Remove(b);
        Emit("{\"ev\":\"button\",\"b\":\"" + b + "\",\"down\":" + (down ? "true" : "false") + "}");
    }

    static IntPtr KeyHook(int code, IntPtr w, IntPtr l)
    {
        if (code < 0 || mode != "capturing") return CallNextHookEx(keyHook, code, w, l);
        var k = (KBDLLHOOKSTRUCT)Marshal.PtrToStructure(l, typeof(KBDLLHOOKSTRUCT));
        if ((k.flags & LLKHF_INJECTED) != 0) return CallNextHookEx(keyHook, code, w, l);
        int msg = w.ToInt32();
        bool down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        if (k.vkCode == VK_SCROLL)
        {
            if (down) Emit("{\"ev\":\"escape\"}"); // acil çıkış
            return new IntPtr(1);
        }
        string id = k.vkCode + ":" + ((k.flags & LLKHF_EXTENDED) != 0 ? 1 : 0);
        if (down) downKeys.Add(id); else downKeys.Remove(id);
        Emit("{\"ev\":\"key\",\"vk\":" + k.vkCode + ",\"sc\":" + k.scanCode + ",\"ext\":" + ((k.flags & LLKHF_EXTENDED) != 0 ? "true" : "false")
             + ",\"down\":" + (down ? "true" : "false") + "}");
        return new IntPtr(1);
    }

    // ---------------- uygulama (SendInput) ----------------
    static void Send(INPUT i) { SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT))); }

    static void InjectMove(double dx, double dy)
    {
        if (Math.Abs(dx) > 5000 || Math.Abs(dy) > 5000) return;
        double nx = vx + dx, ny = vy + dy;
        int l = VL, t = VT, r = VL + VW - 1, b = VT + VH - 1;
        bool leave = (returnEdge == "left" && nx < l) || (returnEdge == "right" && nx > r)
                  || (returnEdge == "top" && ny < t) || (returnEdge == "bottom" && ny > b);
        vx = Math.Max(l, Math.Min(r, nx));
        vy = Math.Max(t, Math.Min(b, ny));
        var mi = new MOUSEINPUT();
        mi.dx = (int)Math.Round((vx - l) * 65535.0 / Math.Max(1, VW - 1));
        mi.dy = (int)Math.Round((vy - t) * 65535.0 / Math.Max(1, VH - 1));
        mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        var i = new INPUT { type = INPUT_MOUSE }; i.u.mi = mi; Send(i);
        if (leave && downButtons.Count == 0)
        {
            var p = new POINT { x = (int)vx, y = (int)vy };
            Emit("{\"ev\":\"leave\",\"pos\":" + PosAlong(returnEdge, p).ToString("0.0000", System.Globalization.CultureInfo.InvariantCulture) + "}");
        }
    }

    static void InjectButton(string b, bool down)
    {
        uint f; uint data = 0;
        switch (b)
        {
            case "left": f = down ? MOUSEEVENTF_LEFTDOWN : MOUSEEVENTF_LEFTUP; break;
            case "right": f = down ? MOUSEEVENTF_RIGHTDOWN : MOUSEEVENTF_RIGHTUP; break;
            case "middle": f = down ? MOUSEEVENTF_MIDDLEDOWN : MOUSEEVENTF_MIDDLEUP; break;
            case "x1": f = down ? MOUSEEVENTF_XDOWN : MOUSEEVENTF_XUP; data = 1; break;
            case "x2": f = down ? MOUSEEVENTF_XDOWN : MOUSEEVENTF_XUP; data = 2; break;
            default: return;
        }
        if (down) downButtons.Add(b); else downButtons.Remove(b);
        var i = new INPUT { type = INPUT_MOUSE }; i.u.mi = new MOUSEINPUT { dwFlags = f, mouseData = data }; Send(i);
    }

    static void InjectWheel(int d, bool h)
    {
        if (d < -12000 || d > 12000) return;
        var i = new INPUT { type = INPUT_MOUSE };
        i.u.mi = new MOUSEINPUT { dwFlags = h ? MOUSEEVENTF_HWHEEL : MOUSEEVENTF_WHEEL, mouseData = unchecked((uint)d) };
        Send(i);
    }

    static void InjectKey(int vk, int sc, bool ext, bool down)
    {
        if (vk <= 0 || vk > 254) return;
        string id = vk + ":" + (ext ? 1 : 0) + ":" + sc;
        if (down) downKeys.Add(id); else downKeys.Remove(id);
        var i = new INPUT { type = INPUT_KEYBOARD };
        i.u.ki = new KEYBDINPUT { wVk = (ushort)vk, wScan = (ushort)sc, dwFlags = (ext ? KEYEVENTF_EXTENDEDKEY : 0) | (down ? 0 : KEYEVENTF_KEYUP) };
        Send(i);
    }

    // Takılı kalan tuş/düğme olmasın: basılı olanların hepsini bırak.
    static void ReleaseAll()
    {
        if (mode == "controlled")
        {
            foreach (var id in new List<string>(downKeys))
            {
                var p = id.Split(':');
                InjectKey(int.Parse(p[0]), int.Parse(p[2]), p[1] == "1", false);
            }
            foreach (var b in new List<string>(downButtons)) InjectButton(b, false);
        }
        downKeys.Clear(); downButtons.Clear();
    }

    static void Cleanup()
    {
        ReleaseAll();
        mode = "idle";
        if (keyHook != IntPtr.Zero) { UnhookWindowsHookEx(keyHook); keyHook = IntPtr.Zero; }
        if (mouseHook != IntPtr.Zero) { UnhookWindowsHookEx(mouseHook); mouseHook = IntPtr.Zero; }
    }
}
