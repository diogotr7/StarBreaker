namespace StarBreaker.P4k;

/// <summary>
/// Represents a P4k-based backing for a P4k structure. This means the p4k structure is stored as an entry in a parent p4k file.
/// </summary>
public sealed class P4kP4kBacking : IP4kBacking
{
    private readonly IP4kFile _p4kFile;
    private readonly P4kEntry _entry;

    public P4kP4kBacking(IP4kFile p4kFile, P4kEntry p4kEntry)
    {
        _p4kFile = p4kFile;
        _entry = p4kEntry;
    }

    public string Name => _entry.Name.Split('\\').Last();

    public Stream Open()
    {
        return _p4kFile.OpenStream(_entry);
    }
}