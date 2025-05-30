namespace StarBreaker.P4k;

/// <summary>
///     Represents a file-based backing for a P4k structure.
/// </summary>
public sealed class FileP4kBacking : IP4kBacking
{
    private readonly string _filePath;

    public FileP4kBacking(string filePath)
    {
        _filePath = filePath;
    }

    public string Name => Path.GetFileName(_filePath);

    public Stream Open()
    {
        return new FileStream(_filePath, FileMode.Open, FileAccess.Read, FileShare.Read);
    }
}