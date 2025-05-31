using Microsoft.Extensions.Logging;
using StarBreaker.FileSystem;
using StarBreaker.P4k;

namespace StarBreaker.Services;

public class P4kService : IP4kService
{
    private readonly ILogger<P4kService> _logger;
    private P4kDirectoryNode? _p4KFile;

    public P4kDirectoryNode P4KFileSystem => _p4KFile ?? throw new InvalidOperationException("P4k file not open");

    public P4kService(ILogger<P4kService> logger)
    {
        _logger = logger;
    }

    public void OpenP4k(string path, IProgress<double> p4kProgress, IProgress<double> fileSystemProgress)
    {
        if (_p4KFile != null)
        {
            _logger.LogWarning("P4k file already open");
            return;
        }

        _p4KFile = P4kDirectoryNode.FromP4k(P4kFile.FromFile(path, p4kProgress), fileSystemProgress);
    }
}