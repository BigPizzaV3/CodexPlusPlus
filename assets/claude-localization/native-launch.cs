using System;
using System.IO;
using System.Text;
using System.ComponentModel;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Threading;

public static class ClaudeNativeLaunch {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo {
        public int cb;
        public string reserved, desktop, title;
        public uint x, y, xSize, ySize, xCount, yCount, fill, flags;
        public ushort show, reservedSize;
        public IntPtr reservedPtr, input, output, error;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo { public IntPtr process, thread; public uint pid, tid; }
    [StructLayout(LayoutKind.Sequential)]
    struct BasicInfo { public IntPtr reserved, peb, reserved2, reserved3, pid, reserved4; }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcessW(string app, StringBuilder args, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string cwd, ref StartupInfo si, out ProcessInfo pi);
    [DllImport("ntdll.dll")]
    static extern int NtQueryInformationProcess(IntPtr process, int cls, ref BasicInfo info, int size, out int returned);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] data, UIntPtr size, out UIntPtr count);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool WriteProcessMemory(IntPtr process, IntPtr address, byte[] data, UIntPtr size, out UIntPtr count);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool VirtualProtectEx(IntPtr process, IntPtr address, UIntPtr size, uint protection, out uint oldProtection);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetExitCodeProcess(IntPtr process, out uint code);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateProcess(IntPtr process, uint code);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool FreeConsole();
    [DllImport("iphlpapi.dll", SetLastError = true)]
    static extern uint GetExtendedTcpTable(IntPtr table, ref uint size, bool order, int family, int tableClass, uint reserved);

    static bool OwnsPort(int port, uint pid) {
        uint size = 0;
        uint result = GetExtendedTcpTable(IntPtr.Zero, ref size, false, 2, 3, 0);
        if (result != 122) throw new Win32Exception((int)result, "Query inspector owner size");
        IntPtr table = Marshal.AllocHGlobal((int)size);
        try {
            result = GetExtendedTcpTable(table, ref size, false, 2, 3, 0);
            if (result != 0) throw new Win32Exception((int)result, "Query inspector owner");
            int count = Marshal.ReadInt32(table);
            for (int i = 0; i < count; i++) {
                int row = 4 + i * 24;
                int localPort = (Marshal.ReadByte(table, row + 8) << 8) | Marshal.ReadByte(table, row + 9);
                if (localPort == port && (uint)Marshal.ReadInt32(table, row + 20) == pid) return true;
            }
            return false;
        } finally { Marshal.FreeHGlobal(table); }
    }

    static void Check(bool success, string label) {
        if (!success) throw new Win32Exception(Marshal.GetLastWin32Error(), label);
    }
    static byte[] Read(IntPtr process, IntPtr address, int length) {
        byte[] data = new byte[length]; UIntPtr count;
        Check(ReadProcessMemory(process, address, data, (UIntPtr)length, out count), "ReadProcessMemory");
        if (count.ToUInt64() != (ulong)length) throw new Exception("Incomplete memory read");
        return data;
    }
    static void WriteFlag(IntPtr process, IntPtr address, byte value) {
        uint previous;
        Check(VirtualProtectEx(process, address, (UIntPtr)1, 4, out previous), "Protect own child fuse");
        try {
            UIntPtr written;
            Check(WriteProcessMemory(process, address, new byte[] { value }, (UIntPtr)1, out written), "Write own child fuse");
            if (written.ToUInt64() != 1 || Read(process, address, 1)[0] != value) throw new Exception("Fuse write not verified");
        } finally {
            uint unused;
            Check(VirtualProtectEx(process, address, (UIntPtr)1, previous, out unused), "Restore page protection");
        }
    }
    static bool PortReady(int port) {
        try {
            using (var client = new TcpClient()) {
                return client.ConnectAsync("127.0.0.1", port).Wait(100) && client.Connected;
            }
        } catch (AggregateException) { return false; }
    }

    // Only a newly created suspended child is touched; disk files and VM service checks stay unchanged.
    public static uint Start(string app, int port) {
        if (IntPtr.Size != 8 || port < 1 || port > 65535) throw new Exception("Native startup requires x64 PowerShell and a valid port");
        if (PortReady(port)) throw new Exception("Inspector port is already occupied");
        byte[] image = File.ReadAllBytes(app);
        if (image.Length < 256 || BitConverter.ToUInt16(image, 0) != 0x5a4d) throw new Exception("Not a PE image");
        int pe = BitConverter.ToInt32(image, 0x3c);
        if (pe < 0 || pe + 24 > image.Length || BitConverter.ToUInt32(image, pe) != 0x4550 || BitConverter.ToUInt16(image, pe + 4) != 0x8664) throw new Exception("Unsupported Claude PE architecture (requires AMD64)");
        string text = Encoding.ASCII.GetString(image), marker = "dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX";
        int offset = -1, start = 0;
        while ((start = text.IndexOf(marker, start, StringComparison.Ordinal)) >= 0) {
            int flags = start + marker.Length + 2;
            if (flags + 4 <= image.Length && image[flags - 2] == 1 && image[flags - 1] >= 4 && flags + image[flags - 1] <= image.Length && (image[flags + 3] == 48 || image[flags + 3] == 49)) {
                if (offset != -1) throw new Exception("Ambiguous Electron fuse marker");
                offset = start;
            }
            start += marker.Length;
        }
        if (offset < 0) throw new Exception("No supported Electron fuse marker");
        int sections = BitConverter.ToUInt16(image, pe + 6), table = pe + 24 + BitConverter.ToUInt16(image, pe + 20);
        long rva = -1;
        for (int i = 0; i < sections; i++) {
            int at = table + 40 * i;
            if (at + 40 > image.Length) throw new Exception("Invalid PE section table");
            uint raw = BitConverter.ToUInt32(image, at + 20), size = BitConverter.ToUInt32(image, at + 16);
            if (offset >= raw && offset + marker.Length + 6 <= (long)raw + size) rva = BitConverter.ToUInt32(image, at + 12) + offset - (long)raw;
        }
        if (rva < 0) throw new Exception("Fuse marker outside mapped sections");
        ProcessInfo pi; StartupInfo si = new StartupInfo(); si.cb = Marshal.SizeOf(si);
        // Electron 会主动 AttachConsole(parent)；仅隐藏窗口或让 PowerShell 退出并不能解除关联。
        Check(FreeConsole(), "Detach launcher console");
        const uint createSuspended = 0x00000004, detachedProcess = 0x00000008;
        Check(CreateProcessW(app, new StringBuilder("\"" + app + "\" --inspect=127.0.0.1:" + port), IntPtr.Zero, IntPtr.Zero, false, createSuspended | detachedProcess, IntPtr.Zero, Path.GetDirectoryName(app), ref si, out pi), "Create detached suspended Claude");
        bool success = false, patched = false;
        IntPtr flag = IntPtr.Zero;
        byte original = image[offset + marker.Length + 5];
        try {
            BasicInfo info = new BasicInfo(); int returned;
            int status = NtQueryInformationProcess(pi.process, 0, ref info, Marshal.SizeOf(info), out returned);
            if (status != 0) throw new Exception("Query process failed: " + status.ToString("X8"));
            long baseAddress = BitConverter.ToInt64(Read(pi.process, IntPtr.Add(info.peb, 16), 8), 0);
            IntPtr address = new IntPtr(baseAddress + rva);
            byte[] current = Read(pi.process, address, marker.Length + 6);
            for (int i = 0; i < current.Length; i++) if (current[i] != image[offset + i]) throw new Exception("Mapped fuse differs from signed file");
            flag = IntPtr.Add(address, marker.Length + 5);
            patched = true;
            WriteFlag(pi.process, flag, 49);
            if (ResumeThread(pi.thread) != 1) throw new Exception("Unexpected suspended thread state");
            var deadline = DateTime.UtcNow.AddSeconds(15);
            while (!PortReady(port)) {
                uint exit;
                Check(GetExitCodeProcess(pi.process, out exit), "Check own child");
                if (exit != 259) throw new Exception("Claude exited before inspector startup");
                if (DateTime.UtcNow >= deadline) throw new Exception("Inspector startup timed out");
                Thread.Sleep(100);
            }
            if (!OwnsPort(port, pi.pid)) throw new Exception("Inspector does not belong to the created Claude process");
            // Node has consumed its startup arguments; do not leave the fuse enabled in memory.
            WriteFlag(pi.process, flag, original);
            patched = false;
            success = true;
            return pi.pid;
        } finally {
            try {
                if (patched) WriteFlag(pi.process, flag, original);
            } finally {
                if (!success) TerminateProcess(pi.process, 1);
                CloseHandle(pi.thread); CloseHandle(pi.process);
            }
        }
    }
}
