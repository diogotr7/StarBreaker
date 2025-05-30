namespace StarBreaker.P4k;

public interface IP4kFile
{
    string Name { get; }
    P4kEntry[] Entries { get; }
    Stream OpenStream(P4kEntry entry);
}