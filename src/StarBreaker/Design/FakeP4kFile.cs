using StarBreaker.P4k;

namespace StarBreaker;

public class FakeP4kFile : IP4kFile
{
    public string Name { get; }
    public P4kEntry[] Entries { get; }

    public FakeP4kFile(string path, P4kEntry[] entries)
    {
        Name = path;
        Entries = entries;
    }

    public Stream OpenStream(P4kEntry entry) => new MemoryStream(new byte[entry.UncompressedSize]);
}