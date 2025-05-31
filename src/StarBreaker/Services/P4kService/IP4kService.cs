using StarBreaker.FileSystem;
using StarBreaker.P4k;

namespace StarBreaker.Services;

public interface IP4kService
{
    P4kDirectoryNode P4KFileSystem { get; }
    void OpenP4k(string path, IProgress<double> p4kProgress, IProgress<double> fileSystemProgress);
}