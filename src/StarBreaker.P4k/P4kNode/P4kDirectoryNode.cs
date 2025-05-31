using System.Diagnostics;
using System.IO.Enumeration;
using System.Runtime.InteropServices;
using StarBreaker.Common;
using StarBreaker.FileSystem;

namespace StarBreaker.P4k;

[DebuggerDisplay("{Name}")]
public sealed class P4kDirectoryNode : IP4kNode, IFileSystem
{
    public P4kRoot Root { get; }
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

    public P4kDirectoryNode(string name, P4kRoot root, IP4kFile p4kFile)
    {
        Name = name;
        Root = root;
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
                value = new P4kDirectoryNode(part.ToString(), Root, file);
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
        var isArchive = p4KEntry.Name.EndsWith(".socpak", StringComparison.OrdinalIgnoreCase) ||
                        p4KEntry.Name.EndsWith(".pak", StringComparison.OrdinalIgnoreCase);

        var isShaderCache = p4KEntry.Name.Contains("shadercache_", StringComparison.OrdinalIgnoreCase);

        if (isArchive && !isShaderCache)
            return FromP4k(P4kFile.FromP4kEntry(p4kFile, p4KEntry));

        return new P4kFileNode(p4KEntry, Root, p4kFile);
    }

    public static P4kDirectoryNode FromP4k(IP4kFile file, IProgress<double>? progress = null)
    {
        progress?.Report(0.0);
        var reportInterval = Math.Max(file.Entries.Length / 500, 1);
        var p4kRoot = new P4kRoot(file);
        var root = new P4kDirectoryNode(file.Name, p4kRoot, file);
        //hacky but this is a recursive reference
        p4kRoot.RootNode = root;

        var entriesProcessed = 0;
        foreach (var entry in file.Entries)
        {
            root.Insert(file, entry);

            entriesProcessed++;
            if (entriesProcessed % reportInterval == 0)
                progress?.Report(entriesProcessed / (double)file.Entries.Length);
        }

        progress?.Report(1.0);

        return root;
    }

    public IEnumerable<string> EnumerateDirectories(string path)
    {
        Span<Range> ranges = stackalloc Range[20];
        var span = path.AsSpan();
        var partsCount = span.Split(ranges, '\\');
        var current = this;

        for (var index = 0; index < partsCount; index++)
        {
            var part = ranges[index];
            if (!current.Children.GetAlternateLookup<ReadOnlySpan<char>>().TryGetValue(span[part], out var value))
                yield break;

            if (value is not P4kDirectoryNode directory)
                yield break;

            current = directory;
        }

        foreach (var child in current.Children.Values.OfType<P4kDirectoryNode>())
        {
            yield return child.Name;
        }
    }

    public IEnumerable<string> EnumerateFiles(string path)
    {
        Span<Range> ranges = stackalloc Range[20];
        var span = path.AsSpan();
        var partsCount = span.Split(ranges, '\\');

        var current = this;

        for (var index = 0; index < partsCount; index++)
        {
            var part = ranges[index];
            if (!current.Children.GetAlternateLookup<ReadOnlySpan<char>>().TryGetValue(span[part], out var value))
                yield break;

            if (value is not P4kDirectoryNode directory)
                yield break;

            current = directory;
        }

        foreach (var child in current.Children.Values.OfType<P4kFileNode>())
        {
            yield return child.P4KEntry.Name;
        }
    }

    public IEnumerable<string> EnumerateFiles(string path, string searchPattern)
    {
        Span<Range> ranges = stackalloc Range[20];
        var span = path.AsSpan();
        var partsCount = span.Split(ranges, '\\');

        var current = this;

        for (var index = 0; index < partsCount; index++)
        {
            var part = ranges[index];
            if (!current.Children.GetAlternateLookup<ReadOnlySpan<char>>().TryGetValue(span[part], out var value))
                yield break;

            if (value is not P4kDirectoryNode directory)
                yield break;

            current = directory;
        }

        foreach (var child in current.Children.Values.OfType<P4kFileNode>())
        {
            if (!FileSystemName.MatchesSimpleExpression(searchPattern, child.P4KEntry.Name.Split('\\').Last()))
                continue;

            yield return child.P4KEntry.Name;
        }
    }

    public bool FileExists(string path)
    {
        Span<Range> ranges = stackalloc Range[20];
        var span = path.AsSpan();
        var partsCount = span.Split(ranges, '\\');
        var current = this;

        for (var index = 0; index < partsCount; index++)
        {
            var part = ranges[index];
            if (!current.Children.GetAlternateLookup<ReadOnlySpan<char>>().TryGetValue(span[part], out var value))
                return false;

            if (value is P4kDirectoryNode directory)
                current = directory;
            else
                return value is P4kFileNode && index == partsCount - 1;
        }

        return false;
    }

    public Stream OpenRead(string path)
    {
        Span<Range> ranges = stackalloc Range[20];
        var span = path.AsSpan();
        var partsCount = span.Split(ranges, '\\');
        var current = this;

        for (var index = 0; index < partsCount; index++)
        {
            var part = ranges[index];
            if (!current.Children.GetAlternateLookup<ReadOnlySpan<char>>().TryGetValue(span[part], out var value))
                throw new FileNotFoundException();

            if (value is P4kDirectoryNode directory)
                current = directory;
            else if (value is P4kFileNode file && index == partsCount - 1)
                return P4k.OpenStream(file.P4KEntry);
            else
                throw new FileNotFoundException();
        }

        throw new FileNotFoundException();
    }

    public byte[] ReadAllBytes(string path)
    {
        Span<Range> ranges = stackalloc Range[20];
        var span = path.AsSpan();
        var partsCount = span.Split(ranges, '\\');
        var current = this;

        for (var index = 0; index < partsCount; index++)
        {
            var part = ranges[index];
            if (!current.Children.GetAlternateLookup<ReadOnlySpan<char>>().TryGetValue(span[part], out var value))
                throw new FileNotFoundException();

            if (value is P4kDirectoryNode directory)
                current = directory;
            else if (value is P4kFileNode file && index == partsCount - 1)
                return P4k.OpenStream(file.P4KEntry).ToArray();
            else
                throw new FileNotFoundException();
        }

        throw new FileNotFoundException();
    }
}