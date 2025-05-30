using System.Diagnostics;
using System.Runtime.InteropServices;

namespace StarBreaker.P4k;

[DebuggerDisplay("{Name}")]
public sealed class P4kDirectoryNode : IP4kNode
{
    private readonly IP4kNode? _parent;
    public IP4kNode Parent => _parent ?? throw new InvalidOperationException("You might have tried to get the parent of the root node");

    public IP4kFile P4k { get; }
    public string Name { get; }
    public Dictionary<string, IP4kNode> Children { get; }

    public ulong Size
    {
        get
        {
            ulong size = 0;
            foreach (var child in Children.Values)
            {
                size += child.Size;
            }

            return size;
        }
    }

    public P4kDirectoryNode(string name, IP4kNode parent, IP4kFile p4kFile)
    {
        Name = name;
        _parent = parent;
        P4k = p4kFile;
        Children = [];
    }

    public void Insert(IP4kFile file, P4kEntry p4KEntry)
    {
        var current = this;
        var name = p4KEntry.Name.AsSpan();

        foreach (var range in name.Split('\\'))
        {
            var part = name[range];
            ref var value = ref CollectionsMarshal.GetValueRefOrAddDefault(current.Children.GetAlternateLookup<ReadOnlySpan<char>>(), part, out var existed);

            if (range.End.Value == name.Length)
            {
                // If this is the last part, we're at the file
                value = GetFromEntry(file, p4KEntry, current);
                return;
            }

            if (!existed)
            {
                value = new P4kDirectoryNode(part.ToString(), current, file);
            }

            if (value is not P4kDirectoryNode directoryNode)
                throw new InvalidOperationException("Expected a directory node");

            current = directoryNode;
        }
    }

    // This is probably suboptimal, but when we do this we'll be doing
    // a lot of IO anyway so it doesn't really matter
    public IEnumerable<P4kEntry> CollectEntries()
    {
        foreach (var child in Children.Values)
        {
            switch (child)
            {
                case P4kDirectoryNode directoryNode:
                    foreach (var entry in directoryNode.CollectEntries())
                        yield return entry;
                    break;
                case P4kFileNode fileNode:
                    yield return fileNode.P4KEntry;
                    break;
                default:
                    throw new Exception();
            }
        }
    }

    private IP4kNode GetFromEntry(IP4kFile p4kFile, P4kEntry p4KEntry, P4kDirectoryNode parent)
    {
        if (p4KEntry.Name.EndsWith(".socpak", StringComparison.OrdinalIgnoreCase))
        {
            var socP4k = P4kFile.FromP4kEntry(p4kFile, p4KEntry);

            return new P4kFileSystem(socP4k).Root;
        }

        return new P4kFileNode(p4KEntry, parent, p4kFile);
    }
}