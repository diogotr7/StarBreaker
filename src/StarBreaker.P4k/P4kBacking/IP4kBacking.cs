namespace StarBreaker.P4k;

/// <summary>
/// Defines the backing store for a p4k structure.
/// Usually this will be a filesystem file, but it can also be a stream, or, most commonly, an socpak that itself is inside a p4k.
/// </summary>
public interface IP4kBacking
{
    string Name { get; }
    Stream Open();
}