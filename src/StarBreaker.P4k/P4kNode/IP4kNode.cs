namespace StarBreaker.P4k;

public interface IP4kNode
{
    IP4kFile P4k { get; }
    P4kRoot Root { get; }
    ulong Size { get; }
}