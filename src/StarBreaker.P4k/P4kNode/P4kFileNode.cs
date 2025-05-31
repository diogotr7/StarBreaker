using System.Diagnostics;

namespace StarBreaker.P4k;

[DebuggerDisplay("{P4KEntry.Name}")]
public sealed class P4kFileNode : IP4kNode
{
    public P4kDirectoryNode Parent { get; }

    public P4kEntry P4KEntry { get; }

    public IP4kFile P4k { get; }

    public ulong Size => P4KEntry.UncompressedSize;

    public P4kFileNode(P4kEntry p4KEntry, P4kDirectoryNode parent, IP4kFile p4kFile)
    {
        P4KEntry = p4KEntry;
        Parent = parent;
        P4k = p4kFile;
    }
}