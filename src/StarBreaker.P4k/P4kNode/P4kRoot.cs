namespace StarBreaker.P4k;

public class P4kRoot
{
    public IP4kFile P4k { get; }
    public P4kDirectoryNode RootNode { get; set;  }
    
    public P4kRoot(IP4kFile p4k)
    {
        P4k = p4k;
    }
}