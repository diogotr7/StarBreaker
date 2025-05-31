using StarBreaker.FileSystem;
using StarBreaker.P4k;

namespace StarBreaker.Services;

public class DesignP4kService : IP4kService
{
    public P4kDirectoryNode P4KFileSystem { get; }

    public DesignP4kService()
    {
        var entries = GetFakeEntries();

        P4KFileSystem = P4kDirectoryNode.FromP4k(new FakeP4kFile(@"C:\This\Is\A\Path", entries));
    }

    public void OpenP4k(string path, IProgress<double> p4kProgress, IProgress<double> fileSystemProgress)
    {
        p4kProgress.Report(0);
        Thread.Sleep(100);
        p4kProgress.Report(0.5);
        Thread.Sleep(100);
        p4kProgress.Report(1);
        Thread.Sleep(100);
        fileSystemProgress.Report(0);
        Thread.Sleep(100);
        fileSystemProgress.Report(0.5);
        Thread.Sleep(100);
        fileSystemProgress.Report(1);
    }

    private static P4kEntry[] GetFakeEntries() =>
    [
        new(@"Data\entry1", 69, 69, 0, false, 123, 0xffff, 0xdeadbeef),
        new(@"Data\entry2", 69, 69, 0, false, 123, 0xffff, 0xdeadbeef),
        new(@"Data\ObjectContainers\entry2", 69, 69, 0, false, 123, 0xffff, 0xdeadbeef),
        new(@"Data\Textures\entry2", 69, 69, 0, false, 123, 0xffff, 0xdeadbeef),
        new(@"Engine\entry3", 69, 69, 0, false, 123, 0xffff, 0xdeadbeef),
        new(@"Engine\entry3", 69, 69, 0, false, 123, 0xffff, 0xdeadbeef)
    ];
}