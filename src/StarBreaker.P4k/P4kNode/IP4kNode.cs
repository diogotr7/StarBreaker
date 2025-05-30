namespace StarBreaker.P4k;

public interface IP4kNode
{
    IP4kFile P4k { get; }
    IP4kNode Parent { get; }
    ulong Size { get; }
}